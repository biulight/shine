//! Shared receipt delivery rules. Persisted DTOs retain their existing wire fields.
use std::path::Path;

pub const LIVE_BUN_LAUNCHER_FORMAT: &str = "live-bun-v2";
pub const NATIVE_CMD_LAUNCHER_FORMAT: &str = "native-cmd-v2";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReceiptMode {
    Snapshot,
    Live,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReceiptRuntime {
    Native,
    Bun,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReceiptDependencies {
    Disabled,
    Locked,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LauncherFormat<'a> {
    Legacy,
    NativeCmdV2,
    LiveBunV2 { config_dir: &'a Path },
}

pub(crate) struct ShellReceiptConfiguration<'a> {
    pub format: Option<&'a str>,
    pub config_dir: Option<&'a Path>,
    pub mode: &'a str,
    pub runtime: &'a str,
    pub source: &'a Path,
    pub transforms: &'a [String],
    pub needs_source: bool,
    pub dependencies: Option<&'a str>,
}
#[derive(Debug)]
pub(crate) struct ValidatedShellConfiguration<'a> {
    pub mode: ReceiptMode,
    pub runtime: ReceiptRuntime,
    pub dependencies: ReceiptDependencies,
    pub format: LauncherFormat<'a>,
}
impl<'a> ShellReceiptConfiguration<'a> {
    pub fn validate(self) -> anyhow::Result<ValidatedShellConfiguration<'a>> {
        use anyhow::bail;
        let mode = match self.mode {
            "snapshot" => ReceiptMode::Snapshot,
            "live" => ReceiptMode::Live,
            _ => bail!("Shell launcher receipt contains an unsupported mode"),
        };
        let runtime = match self.runtime {
            "native" => ReceiptRuntime::Native,
            "bun" => ReceiptRuntime::Bun,
            value => bail!("unsupported Shell launcher runtime in receipt: {value}"),
        };
        let dependencies = match self.dependencies {
            None => ReceiptDependencies::Disabled,
            Some("locked") => ReceiptDependencies::Locked,
            Some(value) => bail!("unsupported Shell Bun dependency mode in receipt: {value}"),
        };
        let format = match (self.format, self.config_dir) {
            (None, None) => LauncherFormat::Legacy,
            (Some(NATIVE_CMD_LAUNCHER_FORMAT), None)
                if runtime == ReceiptRuntime::Native
                    && self.source.extension() == Some(std::ffi::OsStr::new("ps1")) =>
            {
                LauncherFormat::NativeCmdV2
            }
            (Some(LIVE_BUN_LAUNCHER_FORMAT), Some(config_dir))
                if mode == ReceiptMode::Live
                    && runtime == ReceiptRuntime::Bun
                    && !self.transforms.is_empty()
                    && !self.needs_source
                    && config_dir.is_absolute()
                    && !config_dir
                        .components()
                        .any(|part| part == std::path::Component::ParentDir) =>
            {
                LauncherFormat::LiveBunV2 { config_dir }
            }
            _ => bail!("unsupported or inconsistent Shell launcher format in receipt"),
        };
        Ok(ValidatedShellConfiguration {
            mode,
            runtime,
            dependencies,
            format,
        })
    }
}
