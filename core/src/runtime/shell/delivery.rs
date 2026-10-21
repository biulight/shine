//! Pure desired delivery shared by inspection, planning and receipt construction.
use super::*;

pub(in crate::runtime) struct ShellSourceLayout {
    pub source: PathBuf,
    pub rendered: PathBuf,
    pub transforms: Vec<String>,
}

impl ShellSourceLayout {
    pub(in crate::runtime) fn effective_source(&self) -> &Path {
        if self.transforms.is_empty() {
            &self.source
        } else {
            &self.rendered
        }
    }
}

pub(in crate::runtime) struct DesiredShellDelivery {
    pub layout: ShellSourceLayout,
    pub bun: BunRuntimeSpec,
    pub spec: LinkSpec,
    mode: ExternalShellMode,
    needs_source: bool,
}

impl DesiredShellDelivery {
    pub(in crate::runtime) fn launcher_format(&self) -> Option<&'static str> {
        if self.spec.live_launch_config.is_some() {
            Some(LIVE_BUN_LAUNCHER_FORMAT)
        } else if self.spec.native_cmd_literal_path {
            Some(NATIVE_CMD_LAUNCHER_FORMAT)
        } else {
            None
        }
    }

    pub(in crate::runtime) fn receipt_is_current(&self, entry: &ShellManifestEntry) -> bool {
        entry.mode == self.mode
            && entry.source_path == self.layout.source
            && entry.runtime == self.runtime_name()
            && entry.bun_dependencies.as_deref() == self.bun.dependency_mode.as_manifest_value()
            && entry.dependency_hash == self.bun.dependency_hash
            && entry.transforms == self.layout.transforms
            && entry.env == self.spec.env
            && entry.needs_source == self.needs_source
            && entry.launcher_config_dir == self.spec.live_launch_config
            && entry.launcher_format.as_deref() == self.launcher_format()
    }

    fn runtime_name(&self) -> &'static str {
        match self.spec.runtime {
            LinkRuntime::Native => "native",
            LinkRuntime::Bun => "bun",
        }
    }

    pub(super) fn into_receipt(
        self,
        category: &str,
        command: &str,
        mode: ExternalShellMode,
        bytes: &[u8],
    ) -> ShellManifestEntry {
        ShellManifestEntry {
            launcher_format: self.launcher_format().map(str::to_string),
            runtime: self.runtime_name().to_string(),
            launcher_config_dir: self.spec.live_launch_config,
            category: category.to_owned(),
            command: command.to_owned(),
            mode,
            source_path: self.layout.source,
            rendered_path: self.layout.rendered,
            bun_dependencies: self
                .bun
                .dependency_mode
                .as_manifest_value()
                .map(str::to_string),
            dependency_hash: self.bun.dependency_hash,
            transforms: self.layout.transforms,
            env: self.spec.env,
            needs_source: self.needs_source,
            content_hash: crate::install::hash_content(bytes),
        }
    }
}

pub(super) fn source_transforms(file: &ShellFile, bytes: &[u8]) -> Vec<String> {
    if !file.transforms.is_empty() {
        file.transforms.clone()
    } else if has_template_annotation(bytes) {
        vec!["template".to_string()]
    } else {
        Vec::new()
    }
}

impl<H> CoreRuntime<H> {
    pub(in crate::runtime) fn shell_source_layout(
        &self,
        category: &str,
        file: &ShellFile,
        bytes: &[u8],
    ) -> ShellSourceLayout {
        ShellSourceLayout {
            source: self.shell_deployment_source_path(category, &file.source_rel),
            rendered: self.shell_rendered_path(category, &file.source_rel),
            transforms: source_transforms(file, bytes),
        }
    }

    // Build at the caller's original Bun-policy assessment point; this performs no host I/O.
    pub(in crate::runtime) fn desired_shell_delivery(
        &self,
        category: &str,
        file: &ShellFile,
        layout: ShellSourceLayout,
    ) -> Result<DesiredShellDelivery> {
        let bun = self.shell_bun_runtime_spec(category, file)?;
        let spec = LinkSpec {
            native_cmd_literal_path: uses_native_cmd_literal_path(
                file.runtime,
                layout.effective_source(),
            ),
            live_launch_config: self.live_bun_launcher_config(
                file.runtime,
                !layout.transforms.is_empty(),
                file.needs_source,
            ),
            source: layout.effective_source().to_path_buf(),
            link_name: file.command_name.clone().into(),
            runtime: file.runtime,
            bun_dependencies: bun.dependency_mode,
            env: file
                .env
                .iter()
                .map(crate::env::EnvVarSpec::to_with_arg)
                .collect(),
            render_target: (self.context().is_external_presets
                && self.context().external_shell_mode == ExternalShellMode::Live
                && !layout.transforms.is_empty())
            .then(|| format!("shell/{category}/{}", file.command_name)),
        };
        Ok(DesiredShellDelivery {
            layout,
            bun,
            spec,
            mode: self.context().external_shell_mode,
            needs_source: file.needs_source,
        })
    }
}
