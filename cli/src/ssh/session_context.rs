//! Per-session context captured at `shine ssh` time and consulted by the
//! local transfer agent to reconstruct an equivalent connection for the
//! `rsync`/`scp` child (see ADR 0011).
//!
//! Every field here is **local-trusted**: `host`/`ssh_options` come from the
//! user's own `shine ssh` argv (`split_ssh_args`), `local_dir` from the launch
//! cwd, and `control_path` from a socket path shine itself chose. None of it
//! ever originates from the wire, which is why it — and only it — is allowed
//! to shape the `ssh`/`rsync -e` reconnection arguments (a wire-supplied value
//! reaching `-e`/`-o` would be a local command-execution vector).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionContext {
    /// ssh destination exactly as the user gave it (alias or `user@host`).
    pub host: String,
    /// The user's own ssh options, verbatim and in order. Only ever used to
    /// reconnect; never mixed with wire input.
    #[serde(default)]
    pub ssh_options: Vec<String>,
    /// Absolute local working directory captured at `shine ssh` time; the base
    /// against which wire-supplied local paths are resolved.
    pub local_dir: PathBuf,
    /// ControlPath socket for reusing the interactive master connection, when
    /// shine enabled multiplexing (`None` if the user set their own multiplex
    /// options, so we left theirs untouched).
    #[serde(default)]
    pub control_path: Option<PathBuf>,
}

impl SessionContext {
    fn path(session_dir: &Path) -> PathBuf {
        session_dir.join("context.toml")
    }

    pub async fn save(&self, session_dir: &Path) -> Result<()> {
        crate::persist::save_toml_atomic(self, &Self::path(session_dir), "ssh session context")
            .await
    }

    /// Reads back a persisted context. The running agent uses the in-memory
    /// `Arc` instead, so this exists for out-of-process/diagnostic reads of a
    /// live session's `context.toml` (and is exercised by the round-trip test).
    #[allow(dead_code)]
    pub async fn load(session_dir: &Path) -> Result<Self> {
        let path = Self::path(session_dir);
        let content = tokio::fs::read_to_string(&path)
            .await
            .with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&content).context("parsing ssh session context")
    }
}

/// True if the user already configured ssh connection multiplexing themselves,
/// in which case shine must not inject its own `ControlMaster`/`ControlPath`
/// (doing so would fight their settings). Scans for a `-o ControlMaster=`/
/// `-o ControlPath=` option in either the split (`-o`, `ControlPath=…`) or
/// glued (`-oControlPath=…`) forms ssh accepts.
pub fn user_set_control_options(ssh_options: &[String]) -> bool {
    ssh_options.iter().any(|opt| {
        let body = opt.strip_prefix("-o").filter(|rest| !rest.is_empty());
        let value = body.unwrap_or(opt.as_str()).trim();
        let lower = value.to_ascii_lowercase();
        lower.starts_with("controlmaster") || lower.starts_with("controlpath")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_split_control_path_option() {
        let opts = vec![
            "-o".to_string(),
            "ControlPath=/tmp/x".to_string(),
            "dev".to_string(),
        ];
        assert!(user_set_control_options(&opts));
    }

    #[test]
    fn detects_glued_control_master_option() {
        let opts = vec!["-oControlMaster=auto".to_string()];
        assert!(user_set_control_options(&opts));
    }

    #[test]
    fn ignores_unrelated_options() {
        let opts = vec![
            "-p".to_string(),
            "2222".to_string(),
            "-oPort=22".to_string(),
        ];
        assert!(!user_set_control_options(&opts));
    }

    #[tokio::test]
    async fn round_trips_through_toml() {
        let dir = crate::test_support::make_temp_dir("shine-session-context").await;
        let ctx = SessionContext {
            host: "dev".to_string(),
            ssh_options: vec!["-p".to_string(), "2222".to_string()],
            local_dir: PathBuf::from("/home/u/proj"),
            control_path: Some(PathBuf::from("/home/u/.shine/run/ssh/abc/ctl.sock")),
        };
        ctx.save(&dir).await.unwrap();
        let loaded = SessionContext::load(&dir).await.unwrap();
        assert_eq!(ctx, loaded);
        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }
}
