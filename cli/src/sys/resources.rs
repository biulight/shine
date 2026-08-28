use super::{SysDriverKind, SysItem};
use crate::config::Config;
use crate::sys::drivers::{managed_file, split_dns};
use anyhow::{Context, Result, bail};
use std::collections::BTreeMap;
use std::path::Path;
pub(super) use utils::runtime::{
    ManagedFileReceipt, RECEIPT_VERSION, ResourceConflict, ResourceOutcome, ResourcePlan,
    SplitDnsReceipt, SystemReceipt,
};

pub(super) struct DriverContext<'a> {
    pub config: &'a Config,
    pub os_id: &'a str,
    pub item: &'a SysItem,
    pub preset_root: &'a Path,
    pub env: &'a BTreeMap<String, String>,
    pub dry_run: bool,
}

pub(super) trait SystemDriver {
    fn plan(&self, context: &DriverContext<'_>, removing: bool) -> Result<ResourcePlan>;
    async fn apply(
        &self,
        context: &DriverContext<'_>,
        previous: Option<&SystemReceipt>,
    ) -> Result<ResourceOutcome>;
    async fn remove(
        &self,
        context: Option<&DriverContext<'_>>,
        receipt: &SystemReceipt,
        dry_run: bool,
    ) -> Result<ResourceOutcome>;
    /// Cheap, read-only check for whether `apply` would actually change anything.
    /// Used to avoid prompting for admin privileges when a resource is already converged.
    async fn is_up_to_date(
        &self,
        context: &DriverContext<'_>,
        previous: Option<&SystemReceipt>,
    ) -> Result<bool>;
}

pub(super) struct BuiltinDriver {
    kind: SysDriverKind,
}

impl BuiltinDriver {
    pub(super) fn new(kind: SysDriverKind) -> Self {
        Self { kind }
    }

    pub(super) fn update_details(
        &self,
        context: &DriverContext<'_>,
        previous: Option<&SystemReceipt>,
    ) -> Result<Vec<String>> {
        match self.kind {
            SysDriverKind::SplitDns => {
                let desired = split_dns::split_dns_desired(context)?;
                let Some(SystemReceipt::SplitDns(previous)) = previous else {
                    return Ok(vec![
                        "Receipt: missing or incompatible -> desired split DNS state".to_string(),
                    ]);
                };
                let mut details = Vec::new();
                if previous.domain != desired.domain {
                    details.push(format!("Domain: {} -> {}", previous.domain, desired.domain));
                }
                if previous.servers != desired.servers {
                    details.push(format!(
                        "Servers: {} -> {}",
                        previous.servers.join(", "),
                        desired.servers.join(", ")
                    ));
                }
                if previous.resource != desired.resource || previous.os_id != desired.os_id {
                    details.push(format!(
                        "Resource: {} -> {}",
                        previous.resource, desired.resource
                    ));
                }
                if previous.item_id != desired.item_id {
                    details.push(format!("Item: {} -> {}", previous.item_id, desired.item_id));
                }
                Ok(details)
            }
            SysDriverKind::ManagedFile => {
                managed_file::managed_file_update_details(context, previous)
            }
            SysDriverKind::Script => Ok(Vec::new()),
        }
    }
}

impl SystemDriver for BuiltinDriver {
    fn plan(&self, context: &DriverContext<'_>, removing: bool) -> Result<ResourcePlan> {
        match self.kind {
            SysDriverKind::SplitDns => {
                let domain = config_env(context, "domain_env")?;
                let action = if removing { "remove" } else { "converge" };
                let mut description = format!("{action} split DNS for {domain}");
                if context.os_id == "ubuntu"
                    && !removing
                    && !split_dns::systemd_resolved_stub_active()
                {
                    description.push_str(
                        " [WARNING: systemd-resolved's DNS stub (127.0.0.53) looks disabled on \
                         this host, likely because another service holds port 53 (e.g. a \
                         coredns/dnsmasq container) -- the resolved.conf.d drop-in this writes \
                         will have no effect on real lookups until the stub is re-enabled or \
                         /etc/resolv.conf points at whatever is actually resolving DNS here]",
                    );
                }
                Ok(ResourcePlan {
                    description,
                    requires_admin: context.item.requires_admin,
                    restart_hint: None,
                })
            }
            SysDriverKind::ManagedFile => {
                let target = config_string(&context.item.config, "target")?;
                Ok(ResourcePlan {
                    description: format!(
                        "{} managed file {target}",
                        if removing { "remove" } else { "converge" }
                    ),
                    requires_admin: context.item.requires_admin,
                    restart_hint: optional_config_string(&context.item.config, "restart_hint"),
                })
            }
            SysDriverKind::Script => bail!("script is not a built-in system resource driver"),
        }
    }

    async fn apply(
        &self,
        context: &DriverContext<'_>,
        previous: Option<&SystemReceipt>,
    ) -> Result<ResourceOutcome> {
        if let Some(previous) = previous {
            previous.ensure_supported()?;
        }
        match self.kind {
            SysDriverKind::SplitDns => split_dns::apply_split_dns(context, previous).await,
            SysDriverKind::ManagedFile => managed_file::apply_managed_file(context, previous).await,
            SysDriverKind::Script => bail!("script is not a built-in system resource driver"),
        }
    }

    async fn remove(
        &self,
        context: Option<&DriverContext<'_>>,
        receipt: &SystemReceipt,
        dry_run: bool,
    ) -> Result<ResourceOutcome> {
        receipt.ensure_supported()?;
        match (self.kind, receipt) {
            (SysDriverKind::SplitDns, SystemReceipt::SplitDns(receipt)) => {
                split_dns::remove_split_dns(receipt, dry_run).await
            }
            (SysDriverKind::ManagedFile, SystemReceipt::ManagedFile(receipt)) => {
                managed_file::remove_managed_file(receipt, dry_run).await
            }
            (kind, receipt) => bail!(
                "receipt driver mismatch: requested {kind:?}, found {:?}",
                receipt.driver()
            ),
        }
        .with_context(|| {
            context
                .map(|context| format!("processing sys item `{}`", context.item.id))
                .unwrap_or_else(|| "processing recorded system resource".to_string())
        })
    }

    async fn is_up_to_date(
        &self,
        context: &DriverContext<'_>,
        previous: Option<&SystemReceipt>,
    ) -> Result<bool> {
        match self.kind {
            SysDriverKind::SplitDns => split_dns::split_dns_up_to_date(context).await,
            SysDriverKind::ManagedFile => {
                managed_file::managed_file_up_to_date(context, previous).await
            }
            SysDriverKind::Script => Ok(false),
        }
    }
}

pub(super) fn config_string(config: &toml::Table, key: &str) -> Result<String> {
    config
        .get(key)
        .and_then(toml::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .with_context(|| format!("driver config requires non-empty `{key}`"))
}

pub(super) fn optional_config_string(config: &toml::Table, key: &str) -> Option<String> {
    config
        .get(key)
        .and_then(toml::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
}

pub(super) fn config_env(context: &DriverContext<'_>, config_key: &str) -> Result<String> {
    let env_key = config_string(&context.item.config, config_key)?;
    context
        .env
        .get(&env_key)
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .with_context(|| format!("missing environment variable `{env_key}`"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use std::path::PathBuf;

    #[test]
    fn receipt_roundtrips_with_version_and_driver() {
        #[derive(Serialize, Deserialize)]
        struct Wrapper {
            receipt: SystemReceipt,
        }
        let receipt = SystemReceipt::ManagedFile(ManagedFileReceipt {
            version: RECEIPT_VERSION,
            destination: PathBuf::from("/tmp/example"),
            backup: None,
            content_hash: 42,
            privileged: false,
            restart_hint: None,
        });
        let encoded = toml::to_string(&Wrapper {
            receipt: receipt.clone(),
        })
        .unwrap();
        assert!(encoded.contains("driver = \"managed-file\""));
        assert!(encoded.contains("version = 1"));
        let decoded: Wrapper = toml::from_str(&encoded).unwrap();
        assert_eq!(decoded.receipt, receipt);
    }
}
