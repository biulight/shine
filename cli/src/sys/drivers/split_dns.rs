use crate::install_core::file_ops::{UninstallOutcome, install_bytes_admin, uninstall_entry_admin};
use crate::install_core::{AppEntry, AppInstallStrategy, hash_content};
use crate::sys::resources::{
    DriverContext, RECEIPT_VERSION, ResourceOutcome, SplitDnsReceipt, SystemReceipt, config_env,
};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::path::PathBuf;
use std::process::Stdio;
use utils::lifecycle::LifecycleEffect;

const DNS_MARKER_PREFIX: &str = "Managed by shine: split-dns:";

pub(in crate::sys) fn split_dns_desired(context: &DriverContext<'_>) -> Result<SplitDnsReceipt> {
    let domain = normalize_domain(&config_env(context, "domain_env")?)?;
    let servers = normalize_servers(&config_env(context, "servers_env")?)?;
    let resource = match context.os_id {
        "macos" => format!("/etc/resolver/{domain}"),
        "ubuntu" => format!(
            "/etc/systemd/resolved.conf.d/shine-split-dns-{}.conf",
            context.item.id
        ),
        "windows" => format!(".{domain}"),
        other => bail!("split-dns is unsupported on `{other}`"),
    };
    Ok(SplitDnsReceipt {
        version: RECEIPT_VERSION,
        os_id: context.os_id.to_string(),
        item_id: context.item.id.clone(),
        domain,
        servers,
        resource,
        content_hash: None,
    })
}

fn normalize_domain(domain: &str) -> Result<String> {
    let domain = domain.trim().trim_end_matches('.').to_ascii_lowercase();
    let valid = !domain.is_empty()
        && domain.len() <= 253
        && domain.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label
                    .chars()
                    .next()
                    .is_some_and(|character| character.is_ascii_alphanumeric())
                && label
                    .chars()
                    .last()
                    .is_some_and(|character| character.is_ascii_alphanumeric())
                && label
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '-')
        });
    if !valid {
        bail!("invalid private DNS domain `{domain}`");
    }
    Ok(domain)
}

fn normalize_servers(servers: &str) -> Result<Vec<String>> {
    let servers = servers
        .split([',', ' ', '\t', '\n'])
        .map(str::trim)
        .filter(|server| !server.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if servers.is_empty() {
        bail!("PRIVATE_DNS_SERVERS must contain at least one server");
    }
    for server in &servers {
        server
            .parse::<std::net::IpAddr>()
            .with_context(|| format!("invalid DNS server address `{server}`"))?;
    }
    Ok(servers)
}

fn split_dns_marker(item_id: &str) -> String {
    format!("{DNS_MARKER_PREFIX}{item_id}")
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct WindowsNrptRule {
    comment: String,
    namespace: Vec<String>,
    name_servers: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct WindowsNrptRuleQuery {
    rules: Vec<WindowsNrptRule>,
}

fn windows_nrpt_query(marker: &str) -> String {
    format!(
        "$rules=@(Get-DnsClientNrptRule | Where-Object {{$_.Comment -ceq '{marker}'}} | \
         ForEach-Object {{[PSCustomObject]@{{Comment=$_.Comment;Namespace=@($_.Namespace | ForEach-Object {{$_.ToString()}});NameServers=@($_.NameServers | ForEach-Object {{$_.ToString()}})}}}}); \
         [PSCustomObject]@{{Rules=@($rules)}} | ConvertTo-Json -Compress -Depth 3",
        marker = ps_quote(marker),
    )
}

fn windows_nrpt_rules_match_desired(
    rules: WindowsNrptRuleQuery,
    desired: &SplitDnsReceipt,
) -> bool {
    let [rule] = rules.rules.as_slice() else {
        return false;
    };
    rule.comment == split_dns_marker(&desired.item_id)
        && rule.namespace.as_slice() == [desired.resource.as_str()]
        && rule.name_servers == desired.servers
}

async fn windows_split_dns_up_to_date(desired: &SplitDnsReceipt) -> bool {
    let marker = split_dns_marker(&desired.item_id);
    let output = match tokio::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", &windows_nrpt_query(&marker)])
        .output()
        .await
    {
        Ok(output) if output.status.success() => output,
        Ok(_) | Err(_) => return false,
    };
    let Ok(rules) = serde_json::from_slice::<WindowsNrptRuleQuery>(&output.stdout) else {
        return false;
    };
    windows_nrpt_rules_match_desired(rules, desired)
}

/// Whether systemd-resolved's stub listener (127.0.0.53) is actually the thing
/// answering DNS on this host. When `DNSStubListener=no` (e.g. because another
/// service, like a coredns container, holds port 53 instead), the `Domains=`
/// routing rules shine writes under `/etc/systemd/resolved.conf.d/` are never
/// consulted by real lookups: glibc and most applications read `/etc/resolv.conf`
/// directly rather than querying resolved's routing engine. systemd-resolved
/// documents that in that case `/run/systemd/resolve/stub-resolv.conf` becomes an
/// alias for the plain uplink `resolv.conf` (no `nameserver 127.0.0.53` line),
/// which is what we check for here.
const STUB_RESOLV_CONF: &str = "/run/systemd/resolve/stub-resolv.conf";

/// Sync variant for `plan()`, which the `SystemDriver` trait constrains to a
/// non-async fn.
pub(in crate::sys) fn systemd_resolved_stub_active() -> bool {
    stub_active_from(std::fs::read_to_string(STUB_RESOLV_CONF))
}

/// Async variant for `apply_split_dns`, which otherwise does all of its I/O
/// via `tokio::fs` -- avoids blocking the executor thread on this read.
async fn systemd_resolved_stub_active_async() -> bool {
    stub_active_from(tokio::fs::read_to_string(STUB_RESOLV_CONF).await)
}

/// Shared parsing logic for both variants above, factored out so the three
/// outcomes (stub active / stub disabled / file unreadable) are unit-testable
/// without touching the real filesystem.
fn stub_active_from(read_result: std::io::Result<String>) -> bool {
    match read_result {
        Ok(content) => content
            .lines()
            .any(|line| line.trim_start().starts_with("nameserver 127.0.0.53")),
        // Can't verify (non-systemd host, resolved not running yet, etc.) --
        // don't block on something we can't confirm is actually broken.
        Err(_) => true,
    }
}

fn split_dns_file_content(receipt: &SplitDnsReceipt) -> Vec<u8> {
    let marker = split_dns_marker(&receipt.item_id);
    let content = if receipt.os_id == "macos" {
        let servers = receipt
            .servers
            .iter()
            .map(|server| format!("nameserver {server}"))
            .collect::<Vec<_>>()
            .join("\n");
        format!("# {marker}\n{servers}\n")
    } else {
        format!(
            "# {marker}\n[Resolve]\nDNS={}\nDomains=~{}\n",
            receipt.servers.join(" "),
            receipt.domain
        )
    };
    content.into_bytes()
}

pub(in crate::sys) async fn split_dns_up_to_date(context: &DriverContext<'_>) -> Result<bool> {
    let desired = split_dns_desired(context)?;
    if desired.os_id == "windows" {
        return Ok(windows_split_dns_up_to_date(&desired).await);
    }
    let destination = PathBuf::from(&desired.resource);
    if !destination.exists() {
        return Ok(false);
    }
    let current = tokio::fs::read(&destination)
        .await
        .with_context(|| format!("reading {}", destination.display()))?;
    let marker = format!("# {}", split_dns_marker(&desired.item_id));
    if !String::from_utf8_lossy(&current)
        .lines()
        .any(|line| line == marker)
    {
        return Ok(false);
    }
    Ok(current == split_dns_file_content(&desired))
}

pub(in crate::sys) async fn apply_split_dns(
    context: &DriverContext<'_>,
    previous: Option<&SystemReceipt>,
) -> Result<ResourceOutcome> {
    let mut desired = split_dns_desired(context)?;
    if desired.os_id == "ubuntu" && !systemd_resolved_stub_active_async().await {
        bail!(
            "systemd-resolved's DNS stub listener (127.0.0.53) appears to be disabled on this \
             host, likely because another service is bound to port 53 (e.g. a coredns/dnsmasq \
             container). Split DNS routing via /etc/systemd/resolved.conf.d only takes effect \
             when applications query that stub -- with it disabled, this change would be \
             written but silently ineffective. Re-enable `DNSStubListener` in systemd-resolved, \
             or point /etc/resolv.conf at whatever is actually resolving DNS on this host, \
             before retrying."
        );
    }
    if context.dry_run {
        return Ok(ResourceOutcome {
            changed: true,
            effects: vec![LifecycleEffect::ResourceWritePreviewed],
            detail: format!("{} -> {}", desired.domain, desired.servers.join(", ")),
            receipt: Some(SystemReceipt::SplitDns(desired)),
            restart_hint: None,
        });
    }

    let previous_to_remove = match previous {
        Some(SystemReceipt::SplitDns(previous))
            if previous.resource != desired.resource || previous.os_id != desired.os_id =>
        {
            Some(previous)
        }
        _ => None,
    };

    let changed = if desired.os_id == "windows" {
        if windows_split_dns_up_to_date(&desired).await {
            return Ok(ResourceOutcome {
                changed: false,
                effects: Vec::new(),
                detail: desired.domain.clone(),
                receipt: Some(SystemReceipt::SplitDns(desired)),
                restart_hint: None,
            });
        }
        apply_windows_split_dns(&desired).await?;
        true
    } else {
        let destination = PathBuf::from(&desired.resource);
        let content = split_dns_file_content(&desired);
        if destination.exists() {
            let current = tokio::fs::read(&destination)
                .await
                .with_context(|| format!("reading {}", destination.display()))?;
            let marker = format!("# {}", split_dns_marker(&desired.item_id));
            if !String::from_utf8_lossy(&current)
                .lines()
                .any(|line| line == marker)
            {
                bail!(
                    "split DNS destination {} exists but is not owned by shine",
                    destination.display()
                );
            }
            if current == content {
                if desired.os_id == "ubuntu"
                    && !matches!(previous, Some(SystemReceipt::SplitDns(_)))
                {
                    restart_systemd_resolved().await?;
                }
                if let Some(previous) = previous_to_remove {
                    remove_split_dns(previous, false).await?;
                }
                desired.content_hash = Some(hash_content(&content));
                return Ok(ResourceOutcome {
                    changed: false,
                    effects: Vec::new(),
                    detail: desired.domain.clone(),
                    receipt: Some(SystemReceipt::SplitDns(desired)),
                    restart_hint: None,
                });
            }
        }
        install_bytes_admin(&content, &destination, destination.exists(), false, true).await?;
        desired.content_hash = Some(hash_content(&content));
        if desired.os_id == "ubuntu" {
            restart_systemd_resolved().await?;
        }
        if let Some(previous) = previous_to_remove {
            remove_split_dns(previous, false).await?;
        }
        true
    };

    Ok(ResourceOutcome {
        changed,
        effects: if changed {
            vec![LifecycleEffect::ResourceWritten]
        } else {
            Vec::new()
        },
        detail: format!("{} -> {}", desired.domain, desired.servers.join(", ")),
        receipt: Some(SystemReceipt::SplitDns(desired)),
        restart_hint: None,
    })
}

pub(in crate::sys) async fn remove_split_dns(
    receipt: &SplitDnsReceipt,
    dry_run: bool,
) -> Result<ResourceOutcome> {
    if dry_run {
        return Ok(ResourceOutcome {
            changed: true,
            effects: vec![LifecycleEffect::ResourceRemovePreviewed],
            detail: format!("remove split DNS for {}", receipt.domain),
            receipt: None,
            restart_hint: None,
        });
    }
    let changed = if receipt.os_id == "windows" {
        remove_windows_split_dns(receipt).await?
    } else {
        let destination = PathBuf::from(&receipt.resource);
        if !destination.exists() {
            false
        } else {
            let current = tokio::fs::read(&destination)
                .await
                .with_context(|| format!("reading {}", destination.display()))?;
            let marker = format!("# {}", split_dns_marker(&receipt.item_id));
            if !String::from_utf8_lossy(&current)
                .lines()
                .any(|line| line == marker)
            {
                bail!(
                    "refusing to remove non-shine split DNS resource {}",
                    destination.display()
                );
            }
            let entry = AppEntry {
                source: format!("sys/{}/split-dns", receipt.os_id),
                destination: destination.clone(),
                backup: None,
                content_hash: hash_content(&current),
                install_strategy: AppInstallStrategy::Copy,
                uses_env: false,
                requires_admin: true,
            };
            let outcome = uninstall_entry_admin(&entry, false, false).await?;
            if receipt.os_id == "ubuntu" {
                restart_systemd_resolved().await?;
            }
            !matches!(outcome, UninstallOutcome::NotFound)
        }
    };
    Ok(ResourceOutcome {
        changed,
        effects: if changed {
            vec![LifecycleEffect::ResourceRemoved]
        } else {
            Vec::new()
        },
        detail: format!("split DNS for {} removed", receipt.domain),
        receipt: None,
        restart_hint: None,
    })
}

async fn restart_systemd_resolved() -> Result<()> {
    let mut command = if std::env::var("USER").is_ok_and(|user| user == "root") {
        tokio::process::Command::new("systemctl")
    } else {
        let mut command = tokio::process::Command::new("sudo");
        command.args(["-n", "systemctl"]);
        command
    };
    let status = command
        .args(["restart", "systemd-resolved"])
        .status()
        .await
        .context("restarting systemd-resolved")?;
    if !status.success() {
        bail!("failed to restart systemd-resolved");
    }
    Ok(())
}

async fn apply_windows_split_dns(receipt: &SplitDnsReceipt) -> Result<()> {
    let comment = split_dns_marker(&receipt.item_id);
    let servers = receipt
        .servers
        .iter()
        .map(|server| format!("'{}'", ps_quote(server)))
        .collect::<Vec<_>>()
        .join(",");
    let script = format!(
        "$rules=@(Get-DnsClientNrptRule | Where-Object {{$_.Comment -eq '{comment}'}}); \
         foreach($rule in $rules){{Remove-DnsClientNrptRule -Name $rule.Name -Force}}; \
         Add-DnsClientNrptRule -Namespace '{namespace}' -NameServers @({servers}) -Comment '{comment}' | Out-Null",
        comment = ps_quote(&comment),
        namespace = ps_quote(&receipt.resource),
    );
    run_elevated_powershell(&script).await
}

async fn remove_windows_split_dns(receipt: &SplitDnsReceipt) -> Result<bool> {
    let comment = split_dns_marker(&receipt.item_id);
    let script = format!(
        "$rules=@(Get-DnsClientNrptRule | Where-Object {{$_.Comment -eq '{comment}'}}); \
         foreach($rule in $rules){{Remove-DnsClientNrptRule -Name $rule.Name -Force}}",
        comment = ps_quote(&comment),
    );
    run_elevated_powershell(&script).await?;
    Ok(true)
}

fn ps_quote(value: &str) -> String {
    value.replace('\'', "''")
}

async fn run_elevated_powershell(script: &str) -> Result<()> {
    let id = uuid::Uuid::new_v4();
    let script_path = std::env::temp_dir().join(format!("shine-system-{id}.ps1"));
    let result_path = std::env::temp_dir().join(format!("shine-system-{id}.result"));
    let body = format!(
        "$ErrorActionPreference='Stop'\ntry {{\n{script}\nSet-Content -LiteralPath '{result}' -Value 'ok'\nexit 0\n}} catch {{\nSet-Content -LiteralPath '{result}' -Value $_.Exception.Message\nexit 1\n}}\n",
        result = ps_quote(&result_path.display().to_string())
    );
    tokio::fs::write(&script_path, body)
        .await
        .with_context(|| format!("writing {}", script_path.display()))?;
    let arguments = format!(
        "@('-NoProfile','-ExecutionPolicy','Bypass','-File','\"{}\"')",
        ps_quote(&script_path.display().to_string())
    );
    let wrapper = format!(
        "$p=Start-Process powershell.exe -Verb RunAs -Wait -PassThru -ArgumentList {arguments}; exit $p.ExitCode"
    );
    let status = tokio::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", &wrapper])
        .stdin(Stdio::inherit())
        .status()
        .await
        .context("running elevated PowerShell")?;
    let result = tokio::fs::read_to_string(&result_path)
        .await
        .unwrap_or_else(|_| "elevated process did not return a result".to_string());
    let _ = tokio::fs::remove_file(&script_path).await;
    let _ = tokio::fs::remove_file(&result_path).await;
    if !status.success() {
        bail!("elevated PowerShell operation failed: {}", result.trim());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::sys::resources::BuiltinDriver;
    use crate::sys::{SysDriverKind, SysItem, SysItemMode};
    use std::collections::BTreeMap;

    async fn make_temp_dir() -> PathBuf {
        crate::test_support::make_temp_dir("shine-resource").await
    }

    #[test]
    fn stub_active_from_detects_the_stub_nameserver_line() {
        let content = "nameserver 127.0.0.53\noptions edns0\n".to_string();
        assert!(stub_active_from(Ok(content)));
    }

    #[test]
    fn stub_active_from_detects_a_disabled_stub() {
        // When `DNSStubListener=no`, stub-resolv.conf becomes an alias for the
        // plain uplink resolv.conf and has no 127.0.0.53 nameserver line.
        let content = "nameserver 192.0.2.1\n".to_string();
        assert!(!stub_active_from(Ok(content)));
    }

    #[test]
    fn stub_active_from_fails_open_when_unreadable() {
        let error = std::io::Error::new(std::io::ErrorKind::NotFound, "missing");
        assert!(stub_active_from(Err(error)));
    }

    #[test]
    fn split_dns_plans_platform_owned_resources() {
        let dir = std::env::temp_dir();
        let config = Config::new_for_test(&dir);
        let mut driver_config = toml::Table::new();
        driver_config.insert(
            "domain_env".to_string(),
            toml::Value::String("DOMAIN".to_string()),
        );
        driver_config.insert(
            "servers_env".to_string(),
            toml::Value::String("SERVERS".to_string()),
        );
        let item = SysItem {
            id: "private-dns".to_string(),
            label: "Private DNS".to_string(),
            description: String::new(),
            default: false,
            mode: SysItemMode::Managed,
            requires_admin: true,
            required_env: vec!["DOMAIN".to_string(), "SERVERS".to_string()],
            driver: SysDriverKind::SplitDns,
            config: driver_config,
            detect: None,
            install: None,
            shell: Vec::new(),
        };
        let env = BTreeMap::from([
            ("DOMAIN".to_string(), "Home.Example.COM.".to_string()),
            ("SERVERS".to_string(), "10.0.0.2, 10.0.0.3".to_string()),
        ]);
        for (os_id, expected) in [
            ("macos", "/etc/resolver/home.example.com"),
            (
                "ubuntu",
                "/etc/systemd/resolved.conf.d/shine-split-dns-private-dns.conf",
            ),
            ("windows", ".home.example.com"),
        ] {
            let context = DriverContext {
                config: &config,
                os_id,
                item: &item,
                preset_root: &dir,
                env: &env,
                dry_run: true,
            };
            let receipt = split_dns_desired(&context).unwrap();
            assert_eq!(receipt.resource, expected);
            assert_eq!(receipt.servers, ["10.0.0.2", "10.0.0.3"]);
            if os_id != "windows" {
                let content = String::from_utf8(split_dns_file_content(&receipt)).unwrap();
                assert!(content.contains("Managed by shine: split-dns:private-dns"));
            }
        }
    }

    #[tokio::test]
    async fn split_dns_up_to_date_is_false_for_missing_destination() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        let mut driver_config = toml::Table::new();
        driver_config.insert(
            "domain_env".to_string(),
            toml::Value::String("DOMAIN".to_string()),
        );
        driver_config.insert(
            "servers_env".to_string(),
            toml::Value::String("SERVERS".to_string()),
        );
        let item = SysItem {
            id: format!("private-dns-{}", uuid::Uuid::new_v4()),
            label: "Private DNS".to_string(),
            description: String::new(),
            default: false,
            mode: SysItemMode::Managed,
            requires_admin: true,
            required_env: vec!["DOMAIN".to_string(), "SERVERS".to_string()],
            driver: SysDriverKind::SplitDns,
            config: driver_config,
            detect: None,
            install: None,
            shell: Vec::new(),
        };
        let env = BTreeMap::from([
            ("DOMAIN".to_string(), "home.example.com".to_string()),
            ("SERVERS".to_string(), "10.0.0.2".to_string()),
        ]);

        // A fresh item id has never had its resource file written, so it must
        // never be reported as up-to-date (that would skip the admin prompt
        // for a change that hasn't actually been applied yet).
        let ubuntu_context = DriverContext {
            config: &config,
            os_id: "ubuntu",
            item: &item,
            preset_root: &dir,
            env: &env,
            dry_run: false,
        };
        assert!(!split_dns_up_to_date(&ubuntu_context).await.unwrap());

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    fn windows_rule(comment: &str, namespace: &[&str], servers: &[&str]) -> WindowsNrptRule {
        WindowsNrptRule {
            comment: comment.to_string(),
            namespace: namespace.iter().map(ToString::to_string).collect(),
            name_servers: servers.iter().map(ToString::to_string).collect(),
        }
    }

    #[test]
    fn windows_nrpt_rule_must_exactly_match_the_desired_receipt() {
        let desired = SplitDnsReceipt {
            version: RECEIPT_VERSION,
            os_id: "windows".to_string(),
            item_id: "private-dns".to_string(),
            domain: "home.example.com".to_string(),
            servers: vec!["10.0.0.2".to_string(), "10.0.0.3".to_string()],
            resource: ".home.example.com".to_string(),
            content_hash: None,
        };
        let marker = split_dns_marker(&desired.item_id);
        let matching = || windows_rule(&marker, &[".home.example.com"], &["10.0.0.2", "10.0.0.3"]);

        assert!(windows_nrpt_rules_match_desired(
            WindowsNrptRuleQuery {
                rules: vec![matching()]
            },
            &desired
        ));
        assert!(!windows_nrpt_rules_match_desired(
            WindowsNrptRuleQuery {
                rules: vec![windows_rule(
                    &marker,
                    &[".home.example.com"],
                    &["10.0.0.3", "10.0.0.2"]
                )]
            },
            &desired
        ));
        assert!(!windows_nrpt_rules_match_desired(
            WindowsNrptRuleQuery {
                rules: vec![windows_rule(
                    &marker,
                    &[".other.example.com"],
                    &["10.0.0.2", "10.0.0.3"]
                )]
            },
            &desired
        ));
        assert!(!windows_nrpt_rules_match_desired(
            WindowsNrptRuleQuery { rules: Vec::new() },
            &desired
        ));
        assert!(!windows_nrpt_rules_match_desired(
            WindowsNrptRuleQuery {
                rules: vec![matching(), matching()]
            },
            &desired
        ));
    }

    #[test]
    fn windows_nrpt_query_filters_on_the_exact_owned_comment() {
        let marker = split_dns_marker("private-dns");
        let query = windows_nrpt_query(&marker);

        assert!(query.contains("$_.Comment -ceq 'Managed by shine: split-dns:private-dns'"));
        assert!(query.contains("Namespace=@($_.Namespace | ForEach-Object {$_.ToString()})"));
        assert!(query.contains("NameServers=@($_.NameServers | ForEach-Object {$_.ToString()})"));
        assert!(query.contains("Rules=@($rules)"));
    }

    #[test]
    fn split_dns_rejects_invalid_domains_and_servers() {
        for domain in ["", ".example.com", "bad..example", "-bad.example"] {
            assert!(normalize_domain(domain).is_err(), "accepted {domain:?}");
        }
        assert!(normalize_servers("not-an-ip").is_err());
        assert!(normalize_servers(" ").is_err());
    }

    #[test]
    fn split_dns_env_change_requires_update() {
        let dir = std::env::temp_dir();
        let config = Config::new_for_test(&dir);
        let mut driver_config = toml::Table::new();
        driver_config.insert(
            "domain_env".to_string(),
            toml::Value::String("DOMAIN".to_string()),
        );
        driver_config.insert(
            "servers_env".to_string(),
            toml::Value::String("SERVERS".to_string()),
        );
        let item = SysItem {
            id: "split-dns".to_string(),
            label: "Private DNS".to_string(),
            description: String::new(),
            default: false,
            mode: SysItemMode::Managed,
            requires_admin: true,
            required_env: vec!["DOMAIN".to_string(), "SERVERS".to_string()],
            driver: SysDriverKind::SplitDns,
            config: driver_config,
            detect: None,
            install: None,
            shell: Vec::new(),
        };
        let original_env = BTreeMap::from([
            ("DOMAIN".to_string(), "private.example".to_string()),
            ("SERVERS".to_string(), "10.0.0.2".to_string()),
        ]);
        let original_context = DriverContext {
            config: &config,
            os_id: "macos",
            item: &item,
            preset_root: &dir,
            env: &original_env,
            dry_run: true,
        };
        let receipt = SystemReceipt::SplitDns(split_dns_desired(&original_context).unwrap());
        let driver = BuiltinDriver::new(SysDriverKind::SplitDns);
        assert!(
            driver
                .update_details(&original_context, Some(&receipt))
                .unwrap()
                .is_empty()
        );
        let changed_env = BTreeMap::from([
            ("DOMAIN".to_string(), "private.example".to_string()),
            ("SERVERS".to_string(), "10.0.0.3".to_string()),
        ]);
        let changed_context = DriverContext {
            env: &changed_env,
            ..original_context
        };

        assert_eq!(
            driver
                .update_details(&changed_context, Some(&receipt))
                .unwrap(),
            ["Servers: 10.0.0.2 -> 10.0.0.3"]
        );
    }
}
