use super::{
    CoreRuntime, FileSystemHost, LoadedSysPreset, SysDetection, SysDetectionProbe, SysDriverKind,
    SysInstall, SysItem, SysItemMode, SysItemStatus, SysManifest,
};
use anyhow::{Context, Result, bail};
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

pub fn parse_sys_manifest(content: &str) -> Result<SysManifest> {
    let mut manifest: SysManifest = toml::from_str(content)?;
    if let Some(defaults) = &manifest.permission_defaults {
        defaults
            .validate()
            .context("invalid sys permission_defaults")?;
        for item in &mut manifest.items {
            if item.permissions.is_none() {
                item.permissions = Some(defaults.clone());
            }
        }
    }
    validate_sys_manifest(&manifest)?;
    Ok(manifest)
}

pub fn validate_sys_manifest(manifest: &SysManifest) -> Result<()> {
    match manifest.version {
        Some(2) => {}
        None | Some(1) => bail!(
            "sys preset v1 is unsupported; migrate it to version = 2 by removing the monolithic dispatcher, adding detect/install to every init item, and moving software-specific profile code to item integrations. See docs/manual/guides/sys-preset-v2-migration.md"
        ),
        Some(version) => {
            bail!("sys preset version {version} is not supported by this Shine release")
        }
    }
    let mut ids = BTreeSet::new();
    for item in &manifest.items {
        if let Some(permissions) = &item.permissions {
            permissions
                .validate()
                .with_context(|| format!("invalid permissions for sys item `{}`", item.id))?;
        }
        validate_item_id(&item.id)?;
        if item.label.trim().is_empty() {
            bail!("sys bootstrap item `{}` must have a label", item.id);
        }
        if !ids.insert(item.id.clone()) {
            bail!("duplicate sys bootstrap item id `{}`", item.id);
        }
        let mut env = BTreeSet::new();
        for key in &item.required_env {
            validate_env_key(&item.id, key, "required_env")?;
            if !env.insert(key) {
                bail!("sys item `{}` repeats required_env key `{key}`", item.id);
            }
        }
        validate_driver(item)?;
        validate_bootstrap(item)?;
        validate_integrations(item)?;
    }
    if let Some(profile) = &manifest.default_profile
        && !manifest.profiles.contains_key(profile)
    {
        bail!("default profile `{profile}` is not defined");
    }
    for (name, profile) in &manifest.profiles {
        for item_id in &profile.items {
            if !ids.contains(item_id) {
                bail!("profile `{name}` references unknown item `{item_id}`");
            }
            if manifest
                .items
                .iter()
                .find(|item| item.id == *item_id)
                .is_some_and(|item| item.mode == SysItemMode::Managed)
            {
                bail!(
                    "profile `{name}` references managed item `{item_id}`; enable it with `shine sys apply {item_id}`"
                );
            }
        }
    }
    Ok(())
}

impl<H: FileSystemHost> CoreRuntime<H> {
    pub fn available_sys_manifests(&self) -> Result<Vec<(String, SysManifest)>> {
        let os_ids = self
            .presets()
            .files()
            .keys()
            .filter_map(|path| {
                path.strip_prefix("sys/")
                    .and_then(|rest| rest.split_once('/'))
                    .filter(|(_, file)| *file == "shine.toml")
                    .map(|(os_id, _)| os_id.to_string())
            })
            .collect::<BTreeSet<_>>();
        os_ids
            .into_iter()
            .map(|os_id| {
                let logical = format!("sys/{os_id}/shine.toml");
                let bytes = self
                    .presets()
                    .get(&logical)
                    .expect("discovered Sys manifest");
                let content = std::str::from_utf8(bytes).context("sys manifest must be UTF-8")?;
                Ok((os_id, parse_sys_manifest(content)?))
            })
            .collect()
    }

    pub async fn inspect_sys_run_manifest(&self) -> Result<super::SysRunManifest> {
        super::sys::load_manifest_with_host(self.host(), &self.context().shine_dir).await
    }
}

impl<H> CoreRuntime<H> {
    pub async fn load_sys_preset(&self, os_id: &str) -> Result<LoadedSysPreset> {
        if os_id.contains(['/', '\\']) || os_id.contains("..") {
            bail!("invalid os id: {os_id:?}");
        }
        let prefix = format!("sys/{os_id}/");
        let manifest_path = format!("{prefix}shine.toml");
        let bytes = self
            .presets()
            .get(&manifest_path)
            .with_context(|| format!("reading {manifest_path}"))?;
        let content = std::str::from_utf8(bytes).context("sys manifest must be UTF-8")?;
        let manifest =
            parse_sys_manifest(content).with_context(|| format!("parsing {manifest_path}"))?;
        // This path is presentation-only until an executing path explicitly
        // materializes the immutable snapshot. Domain reads continue to use
        // snapshot bytes and never reopen this ambient directory.
        let root = self
            .presets()
            .origin(&manifest_path)
            .and_then(|origin| origin.category_root.clone())
            .unwrap_or_else(|| self.context().presets_dir.join("sys").join(os_id));
        Ok(LoadedSysPreset { manifest, root })
    }
}

impl<H: FileSystemHost> CoreRuntime<H> {
    /// Materialize a private invocation; never replace another running snapshot.
    /// Inspection, preview and profile composition must not call this method.
    pub(crate) async fn materialize_sys_preset(&self, os_id: &str) -> Result<PathBuf> {
        if os_id.is_empty() || os_id.contains(['/', '\\']) || os_id.contains("..") {
            bail!("invalid os id: {os_id:?}");
        }
        let prefix = format!("sys/{os_id}/");
        let root = self
            .context()
            .shine_dir
            .join("runtime/sys")
            .join(os_id)
            .join(uuid::Uuid::new_v4().to_string());
        let result = async {
            self.host()
                .create_dir_all(&root)
                .await
                .map_err(|error| error.into_anyhow("creating Sys execution snapshot"))?;
            self.host()
                .set_mode(&root, 0o700)
                .await
                .map_err(|error| error.into_anyhow("protecting Sys execution snapshot"))?;
            for (logical, bytes) in self
                .presets()
                .files()
                .iter()
                .filter(|(path, _)| path.starts_with(&prefix))
            {
                let relative = logical
                    .strip_prefix(&prefix)
                    .expect("filtered Sys snapshot path");
                let path = safe_snapshot_path(&root, relative)?;
                self.host()
                    .write_atomic(&path, bytes)
                    .await
                    .map_err(|error| error.into_anyhow("materializing Sys execution snapshot"))?;
                if self
                    .presets()
                    .file(logical)
                    .is_some_and(|file| file.executable)
                {
                    self.host().set_executable(&path).await.map_err(|error| {
                        error.into_anyhow("making Sys snapshot helper executable")
                    })?;
                }
            }
            Ok::<_, anyhow::Error>(())
        }
        .await;
        if let Err(error) = result {
            let _ = self.host().remove_dir_all(&root).await;
            return Err(error);
        }
        Ok(root)
    }
}

fn safe_snapshot_path(root: &Path, relative: &str) -> Result<PathBuf> {
    if relative.is_empty()
        || !Path::new(relative)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
    {
        bail!("Sys snapshot path must stay inside its category");
    }
    Ok(root.join(relative))
}

fn validate_driver(item: &SysItem) -> Result<()> {
    if item.mode == SysItemMode::Init && item.driver != SysDriverKind::Script {
        bail!(
            "sys bootstrap item `{}` cannot use managed driver `{:?}`",
            item.id,
            item.driver
        );
    }
    let allowed: &[&str] = match item.driver {
        SysDriverKind::Script => &[],
        SysDriverKind::SplitDns => &["domain_env", "servers_env"],
        SysDriverKind::ManagedFile => &["source", "target", "transforms", "restart_hint"],
    };
    for key in item.config.keys() {
        if !allowed.contains(&key.as_str()) {
            bail!(
                "sys item `{}` has unknown {:?} driver config key `{key}`",
                item.id,
                item.driver
            );
        }
    }
    match item.driver {
        SysDriverKind::Script => {
            if !item.config.is_empty() {
                bail!(
                    "script sys item `{}` does not accept driver config",
                    item.id
                );
            }
        }
        SysDriverKind::SplitDns => {
            for key in ["domain_env", "servers_env"] {
                let env_key = config_string(&item.config, key)?;
                if !item.required_env.contains(&env_key) {
                    bail!(
                        "sys item `{}` config `{key}` references `{env_key}` but required_env does not include it",
                        item.id
                    );
                }
            }
        }
        SysDriverKind::ManagedFile => {
            config_string(&item.config, "source")?;
            config_string(&item.config, "target")?;
            if let Some(values) = item.config.get("transforms") {
                let values = values.as_array().with_context(|| {
                    format!(
                        "sys item `{}` config `transforms` must be an array",
                        item.id
                    )
                })?;
                if values.iter().any(|value| value.as_str().is_none()) {
                    bail!(
                        "sys item `{}` config `transforms` must contain strings",
                        item.id
                    );
                }
            }
        }
    }
    Ok(())
}

fn validate_bootstrap(item: &SysItem) -> Result<()> {
    if item.mode == SysItemMode::Managed {
        if item.detect.is_some() || item.install.is_some() || !item.shell.is_empty() {
            bail!(
                "managed sys item `{}` cannot declare bootstrap detect/install/shell fields",
                item.id
            );
        }
        return Ok(());
    }
    let detect = item
        .detect
        .as_ref()
        .with_context(|| format!("sys bootstrap item `{}` must declare `detect`", item.id))?;
    let install = item
        .install
        .as_ref()
        .with_context(|| format!("sys bootstrap item `{}` must declare `install`", item.id))?;
    validate_detection(&item.id, detect)?;
    match install {
        SysInstall::Package {
            package,
            success_status,
            success_hint,
            ..
        } => {
            validate_plain(&item.id, "package", package)?;
            if package.starts_with('-')
                || !package
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-'))
            {
                bail!(
                    "sys item `{}` has invalid package identifier `{package}`",
                    item.id
                );
            }
            validate_success(&item.id, *success_status)?;
            if !success_hint.is_empty() {
                validate_plain(&item.id, "success hint", success_hint)?;
            }
        }
        SysInstall::Script {
            path,
            success_status,
            success_hint,
        } => {
            validate_relative(&item.id, "install script", path)?;
            validate_success(&item.id, *success_status)?;
            if !success_hint.is_empty() {
                validate_plain(&item.id, "success hint", success_hint)?;
            }
        }
    }
    Ok(())
}

fn validate_detection(id: &str, detection: &SysDetection) -> Result<()> {
    match detection {
        SysDetection::Command {
            command,
            version_args,
        } => {
            validate_command(id, command)?;
            for arg in version_args {
                validate_plain(id, "version argument", arg)?;
            }
        }
        SysDetection::Path { path } => validate_plain(id, "detection path", path)?,
        SysDetection::Any { probes } => {
            if probes.is_empty() {
                bail!("sys item `{id}` detection `any` requires at least one probe");
            }
            for probe in probes {
                match probe {
                    SysDetectionProbe::Command { command } => validate_command(id, command)?,
                    SysDetectionProbe::Path { path } => validate_plain(id, "detection path", path)?,
                }
            }
        }
    }
    Ok(())
}

fn validate_integrations(item: &SysItem) -> Result<()> {
    for (index, integration) in item.shell.iter().enumerate() {
        if integration.shells.is_empty() {
            bail!(
                "sys item `{}` shell integration {} requires at least one shell",
                item.id,
                index + 1
            );
        }
        if let Some(command) = &integration.when_command {
            validate_command(&item.id, command)?;
        }
        let count = usize::from(integration.path.is_some())
            + usize::from(!integration.env.is_empty())
            + usize::from(!integration.eval_argv.is_empty())
            + usize::from(integration.source.is_some())
            + usize::from(!integration.aliases.is_empty())
            + usize::from(integration.fragment.is_some());
        if count != 1 {
            bail!(
                "sys item `{}` shell integration {} must declare exactly one of path, env, eval, source, aliases, or fragment",
                item.id,
                index + 1
            );
        }
        if let Some(path) = &integration.path {
            validate_plain(&item.id, "profile path", path)?;
        }
        for (key, value) in &integration.env {
            validate_env_key(&item.id, key, "profile env")?;
            validate_plain(&item.id, "profile env value", value)?;
        }
        for arg in &integration.eval_argv {
            validate_plain(&item.id, "profile eval argument", arg)?;
        }
        if let Some(source) = &integration.source {
            validate_plain(&item.id, "profile source", source)?;
        }
        for (name, value) in &integration.aliases {
            if name.is_empty()
                || !name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
            {
                bail!("sys item `{}` has invalid alias name `{name}`", item.id);
            }
            validate_plain(&item.id, "profile alias", value)?;
        }
        if let Some(fragment) = &integration.fragment {
            validate_relative(&item.id, "profile fragment", fragment)?;
        }
    }
    Ok(())
}

fn validate_success(id: &str, status: Option<SysItemStatus>) -> Result<()> {
    if status.is_some_and(|status| {
        !matches!(
            status,
            SysItemStatus::Installed | SysItemStatus::NeedsAction
        )
    }) {
        bail!("sys item `{id}` install success_status must be installed or needs-action");
    }
    Ok(())
}
fn validate_item_id(id: &str) -> Result<()> {
    if id.trim().is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
    {
        bail!(
            "sys bootstrap item id `{id}` contains invalid characters (allowed: a-z A-Z 0-9 - _)"
        );
    }
    Ok(())
}
fn validate_command(id: &str, value: &str) -> Result<()> {
    if value.is_empty()
        || !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '+'))
    {
        bail!("sys item `{id}` has invalid command name `{value}`");
    }
    Ok(())
}
fn validate_env_key(id: &str, key: &str, label: &str) -> Result<()> {
    let mut chars = key.chars();
    if !chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        bail!("sys item `{id}` has invalid {label} key `{key}`");
    }
    Ok(())
}
fn validate_plain(id: &str, label: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() || value.chars().any(char::is_control) {
        bail!("sys item `{id}` has invalid {label}");
    }
    Ok(())
}
fn validate_relative(id: &str, label: &str, value: &str) -> Result<()> {
    validate_plain(id, label, value)?;
    let path = Path::new(value);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        bail!("sys item `{id}` {label} must stay inside the preset: `{value}`");
    }
    Ok(())
}
fn config_string(config: &toml::Table, key: &str) -> Result<String> {
    config
        .get(key)
        .and_then(toml::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .with_context(|| format!("driver config requires non-empty `{key}`"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{OpaqueCodeScopeV1, PermissionV1};
    use crate::runtime::{
        FileSystemObservationHost, HostOperation, InMemoryHost, PresetSnapshot, PresetSourceKind,
        RuntimeContext, RuntimePlatform,
    };

    #[test]
    fn sys_permission_defaults_resolve_per_item_with_explicit_override() {
        let manifest = parse_sys_manifest(
            r#"
version = 2
[permission_defaults]
schema_version = 2
opaque_code = "unrestricted"

[[items]]
id = "one"
label = "One"
detect = { kind = "path", path = "~/one" }
install = { kind = "script", path = "one.sh" }

[[items]]
id = "two"
label = "Two"
detect = { kind = "path", path = "~/two" }
install = { kind = "script", path = "two.sh" }
[items.permissions]
schema_version = 1
commands = ["git"]
"#,
        )
        .unwrap();

        let inherited = manifest.items[0].permissions.as_ref().unwrap();
        let overridden = manifest.items[1].permissions.as_ref().unwrap();
        assert!(
            inherited
                .permission_set()
                .unwrap()
                .contains(&PermissionV1::OpaqueCode {
                    scope: OpaqueCodeScopeV1::Unrestricted,
                })
        );
        assert!(
            overridden
                .permission_set()
                .unwrap()
                .contains(&PermissionV1::Command {
                    program: "git".to_string(),
                })
        );
        assert!(
            !overridden
                .permission_set()
                .unwrap()
                .contains(&PermissionV1::OpaqueCode {
                    scope: OpaqueCodeScopeV1::Unrestricted,
                })
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn running_sys_snapshot_retains_its_helpers_when_another_is_materialized() {
        use crate::runtime::{ProcessHost, ProcessRequest, RealHost};
        use std::os::unix::fs::PermissionsExt;
        use std::time::Duration;
        let root =
            std::env::temp_dir().join(format!("shine-sys-snapshot-{}", uuid::Uuid::new_v4()));
        let context = RuntimeContext::isolated(
            root.clone(),
            root.join("state"),
            root.join("presets"),
            root.join("bin"),
            RuntimePlatform::current(),
        );
        let snapshot = |value: &str| {
            PresetSnapshot::builder(PresetSourceKind::External)
            .file("sys/test/install.sh", b"printf ready > \"$1\"; while [ ! -e \"$2\" ]; do sleep 0.01; done; ./helper\n".to_vec())
            .file_with_executable("sys/test/helper", format!("#!/bin/sh\nprintf '{value}'\n").into_bytes(), true)
            .file("sys/test/data", b"data".to_vec()).build()
        };
        let first = CoreRuntime::new(RealHost, context.clone(), snapshot("captured-first"));
        let second = CoreRuntime::new(RealHost, context, snapshot("captured-second"));
        let first_root = first.materialize_sys_preset("test").await.unwrap();
        let ready = root.join("ready");
        let release = root.join("release");
        let process = tokio::spawn(RealHost.run(ProcessRequest {
            program: "sh".into(),
            args: vec![
                first_root.join("install.sh").display().to_string(),
                ready.display().to_string(),
                release.display().to_string(),
            ],
            cwd: Some(first_root.clone()),
            timeout: Some(Duration::from_secs(5)),
            ..Default::default()
        }));
        tokio::time::timeout(Duration::from_secs(5), async {
            while !ready.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let second_root = second.materialize_sys_preset("test").await.unwrap();
        assert_ne!(first_root, second_root);
        let second_output = RealHost
            .run(ProcessRequest {
                program: second_root.join("helper").display().to_string(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(second_output.stdout, b"captured-second");
        std::fs::write(release, b"go").unwrap();
        let output = process.await.unwrap().unwrap();
        assert_eq!(output.exit_code, Some(0));
        assert_eq!(output.stdout, b"captured-first");
        assert_eq!(
            std::fs::metadata(&first_root).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(first_root.join("data"))
                .unwrap()
                .permissions()
                .mode()
                & 0o111,
            0
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn external_sys_preset_load_is_read_only_and_execution_materializes_snapshot() {
        let host = InMemoryHost::new();
        let context = RuntimeContext::isolated(
            "/virtual/home".into(),
            "/virtual/home/.shine".into(),
            "/ambient/presets".into(),
            "/virtual/home/.shine/bin".into(),
            RuntimePlatform::Linux,
        );
        let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
            .base_root("/ambient/presets")
            .file(
                "sys/test/shine.toml",
                b"version = 2\n[[items]]\nid = \"demo\"\nlabel = \"Demo\"\ndetect = { kind = \"path\", path = \"~/demo\" }\ninstall = { kind = \"script\", path = \"install.sh\" }\n"
                    .to_vec(),
            )
            .file("sys/test/install.sh", b"echo captured\n".to_vec())
            .build();
        let runtime = CoreRuntime::new(host.clone(), context, snapshot);

        let loaded = runtime.load_sys_preset("test").await.unwrap();

        assert_eq!(loaded.root, Path::new("/ambient/presets/sys/test"));
        assert!(
            !host.operations().iter().any(|operation| matches!(
                operation,
                HostOperation::Write(_)
                    | HostOperation::CreateDirectory(_)
                    | HostOperation::Remove(_)
                    | HostOperation::RemoveDirectory(_)
            )),
            "loading a Sys preset must remain read-only"
        );
        assert!(
            host.read(Path::new(
                "/virtual/home/.shine/runtime/sys/test/install.sh"
            ))
            .await
            .is_err()
        );
        host.put_file(
            "/virtual/home/.shine/runtime/sys/test/stale.sh",
            b"stale".to_vec(),
        );

        let execution_root = runtime.materialize_sys_preset("test").await.unwrap();
        assert_eq!(
            execution_root.parent(),
            Some(Path::new("/virtual/home/.shine/runtime/sys/test"))
        );
        assert_eq!(
            host.read(&execution_root.join("install.sh")).await.unwrap(),
            b"echo captured\n"
        );
        assert!(host.read(&execution_root.join("stale.sh")).await.is_err());
        assert!(
            host.read(Path::new("/ambient/presets/sys/test/install.sh"))
                .await
                .is_err()
        );
    }
}
