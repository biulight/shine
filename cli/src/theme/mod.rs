//! `shine theme sync`: resolves the terminal's light/dark theme and prints
//! shell-safe export statements for `SHINE_TERMINAL_THEME` and `BAT_THEME`.
//! See docs/terminal-theme-sync-prd.md for the full design and
//! docs/kb/lessons.md (2026-07-14) for why the old shell-only OSC read loop
//! was replaced.

mod color;
#[cfg(unix)]
mod osc;

use std::time::Duration;

use anyhow::Result;

use crate::config::Config;
use crate::env::commands::format_env_export;

pub use color::{Theme, parse_colorfgbg, parse_theme_str};

/// Total time budget for an OSC 11 round trip. Deliberately generous
/// relative to the sub-millisecond local RTT measured during the PRD's
/// investigation, since it must also tolerate a genuinely slow/lossy SSH
/// link without hanging shell startup (PRD §11: 200ms cap on "terminal
/// unresponsive").
const OSC_QUERY_BUDGET: Duration = Duration::from_millis(200);

#[cfg(unix)]
fn query_terminal_theme(budget: Duration) -> Option<Theme> {
    osc::query_terminal_theme(budget)
}

#[cfg(not(unix))]
fn query_terminal_theme(_budget: Duration) -> Option<Theme> {
    None
}

/// Resolves the local terminal's theme for injection into a `shine ssh`
/// remote session (PRD §6.1): prefers an already-exported
/// `SHINE_TERMINAL_THEME`, otherwise queries the local tty directly — unlike
/// a remote query, this is a same-host round trip with no fragmentation risk
/// (PRD §2.2). Returns `None` rather than failing `shine ssh` itself: this
/// is a display-layer nicety, never a reason to block a login.
pub fn resolve_local_terminal_theme_for_injection() -> Option<Theme> {
    if let Ok(existing) = std::env::var("SHINE_TERMINAL_THEME")
        && let Some(theme) = parse_theme_str(existing.trim())
    {
        return Some(theme);
    }
    query_terminal_theme(OSC_QUERY_BUDGET)
}

/// `true` when auto-sync (profile-driven) should proceed: an explicit
/// `SHINE_SYNC_TERMINAL_THEME` env var always wins over config (PRD §5:
/// "环境变量...覆盖配置文件"); when unset, falls back to
/// `config.sync_terminal_theme`. Manual invocations (`auto: false` in
/// [`handle_sync`]) never call this — PRD §5: "手动同步命令不受该开关限制".
fn auto_sync_enabled(config: &Config) -> bool {
    match std::env::var("SHINE_SYNC_TERMINAL_THEME") {
        Ok(value) => value.trim() != "0",
        Err(_) => config.sync_terminal_theme,
    }
}

/// Resolves the `BAT_THEME` value for `theme`, honoring the already-published
/// `SHINE_BAT_LIGHT_THEME`/`SHINE_BAT_DARK_THEME` overrides (PRD §5.1) with
/// their existing defaults.
fn resolve_bat_theme_override(theme: Theme) -> String {
    let (var, default) = match theme {
        Theme::Light => ("SHINE_BAT_LIGHT_THEME", "GitHub"),
        Theme::Dark => ("SHINE_BAT_DARK_THEME", "OneHalfDark"),
    };
    std::env::var(var)
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default.to_string())
}

/// Priority chain from PRD §6: an already-exported `SHINE_TERMINAL_THEME`
/// (set by the user, a parent shell's own sync, or `shine ssh`'s injection)
/// wins outright with no tty interaction; then `COLORFGBG`; then an OSC 11
/// query. Returns `None` if nothing resolves.
fn resolve_theme() -> Option<Theme> {
    if let Ok(existing) = std::env::var("SHINE_TERMINAL_THEME")
        && let Some(theme) = parse_theme_str(existing.trim())
    {
        return Some(theme);
    }
    if let Ok(colorfgbg) = std::env::var("COLORFGBG")
        && let Some(theme) = parse_colorfgbg(&colorfgbg)
    {
        return Some(theme);
    }
    query_terminal_theme(OSC_QUERY_BUDGET)
}

/// `shine theme sync [--auto] [--quiet]`. Prints `eval`-able shell export
/// statements to stdout; diagnostics go to stderr (suppressed by
/// `--quiet`). Always exits successfully — an unresolved theme prints
/// nothing rather than failing, so an old/broken binary or an unsupported
/// terminal never blocks shell startup (PRD §7).
pub async fn handle_sync(auto: bool, quiet: bool) -> Result<()> {
    // Read-only: this runs on every interactive shell start and must never
    // create shine state on disk (AGENTS.md: Config::load_or_init() writes
    // to disk even for read-oriented commands).
    let config = Config::load_global_runtime_for_dry_run().await?;

    if auto && !auto_sync_enabled(&config) {
        return Ok(());
    }

    let Some(theme) = resolve_theme() else {
        if !quiet {
            eprintln!("shine: could not determine terminal theme; leaving BAT_THEME unchanged");
        }
        return Ok(());
    };

    println!(
        "{}",
        format_env_export(&config.shell_type, "SHINE_TERMINAL_THEME", theme.as_str())
    );

    // Preserve a BAT_THEME the user (or anything other than shine) already
    // set, regardless of its source — this is also what makes a nested
    // shell that inherited a parent's already-shine-set BAT_THEME a no-op
    // here (PRD §6.5, a deliberate behavior change from the old
    // unconditional overwrite in profile.pre.sh).
    if std::env::var("BAT_THEME").is_ok() {
        return Ok(());
    }
    let bat_theme = resolve_bat_theme_override(theme);
    println!(
        "{}",
        format_env_export(&config.shell_type, "BAT_THEME", &bat_theme)
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::env_lock;

    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "current_thread")]
    async fn handle_sync_skips_when_auto_and_env_var_disabled() {
        let _guard = env_lock();
        // SAFETY: env_lock() is held for the duration of this block.
        unsafe { std::env::set_var("SHINE_SYNC_TERMINAL_THEME", "0") };
        unsafe { std::env::set_var("SHINE_TERMINAL_THEME", "dark") };

        let result = handle_sync(true, true).await;
        assert!(result.is_ok());

        unsafe { std::env::remove_var("SHINE_SYNC_TERMINAL_THEME") };
        unsafe { std::env::remove_var("SHINE_TERMINAL_THEME") };
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "current_thread")]
    async fn handle_sync_manual_ignores_env_disable() {
        let _guard = env_lock();
        unsafe { std::env::set_var("SHINE_SYNC_TERMINAL_THEME", "0") };
        unsafe { std::env::set_var("SHINE_TERMINAL_THEME", "dark") };
        unsafe { std::env::remove_var("BAT_THEME") };

        // auto = false: PRD §5 says manual sync must not be gated by
        // SHINE_SYNC_TERMINAL_THEME/config, so this must still resolve
        // rather than short-circuiting through the auto-gate.
        let result = handle_sync(false, true).await;
        assert!(result.is_ok());

        unsafe { std::env::remove_var("SHINE_SYNC_TERMINAL_THEME") };
        unsafe { std::env::remove_var("SHINE_TERMINAL_THEME") };
    }

    #[test]
    fn resolve_bat_theme_override_uses_published_env_vars() {
        let _guard = env_lock();
        // SAFETY: env_lock() is held for the duration of this block.
        unsafe { std::env::set_var("SHINE_BAT_LIGHT_THEME", "Solarized") };
        assert_eq!(resolve_bat_theme_override(Theme::Light), "Solarized");
        unsafe { std::env::remove_var("SHINE_BAT_LIGHT_THEME") };

        assert_eq!(resolve_bat_theme_override(Theme::Light), "GitHub");
        assert_eq!(resolve_bat_theme_override(Theme::Dark), "OneHalfDark");
    }

    #[test]
    fn resolve_bat_theme_override_ignores_empty_env_var() {
        let _guard = env_lock();
        unsafe { std::env::set_var("SHINE_BAT_DARK_THEME", "") };
        assert_eq!(resolve_bat_theme_override(Theme::Dark), "OneHalfDark");
        unsafe { std::env::remove_var("SHINE_BAT_DARK_THEME") };
    }

    #[test]
    fn resolve_theme_prefers_already_exported_var_over_colorfgbg() {
        let _guard = env_lock();
        unsafe { std::env::set_var("SHINE_TERMINAL_THEME", "light") };
        unsafe { std::env::set_var("COLORFGBG", "15;0") }; // would resolve dark
        assert_eq!(resolve_theme(), Some(Theme::Light));
        unsafe { std::env::remove_var("SHINE_TERMINAL_THEME") };
        unsafe { std::env::remove_var("COLORFGBG") };
    }

    #[test]
    fn resolve_theme_falls_back_to_colorfgbg() {
        let _guard = env_lock();
        unsafe { std::env::remove_var("SHINE_TERMINAL_THEME") };
        unsafe { std::env::set_var("COLORFGBG", "0;15") };
        assert_eq!(resolve_theme(), Some(Theme::Light));
        unsafe { std::env::remove_var("COLORFGBG") };
    }
}
