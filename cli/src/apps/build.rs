use super::metadata::{self, AppCategory};
use crate::colors;
use crate::config::Config;
use anyhow::{Context, Result, bail};
use directories::BaseDirs;
use tokio::fs;
use tokio::process::Command;

/// Runs the `[artifact].script` declared by an app preset (`shine app artifact apply <app-id>`).
///
/// Unlike `post_upgrade` hooks (which are a background side effect of `shine upgrade` and
/// swallow failures so one broken hook doesn't abort the whole upgrade), this is a single
/// explicit user action: script failures propagate as a real error, and output streams live
/// instead of being captured, so the user can see a build fail as it happens.
pub async fn handle_build(config: &Config, app_id: &str) -> Result<()> {
    let categories = metadata::load_active_categories(config, Some(app_id)).await?;
    let cat = categories
        .iter()
        .find(|c| c.name == app_id)
        .ok_or_else(|| anyhow::anyhow!("app preset category not found: {app_id}"))?;

    let Some(artifact) = &cat.artifact else {
        bail!("app '{app_id}' does not define an artifact script");
    };

    let mut command = artifact_command(config, app_id, &artifact.script, artifact.runtime).await?;
    run_artifact_command(&mut command, app_id).await
}

/// Runs the `[artifact].teardown` script (`shine app artifact remove <app-id>`), the
/// symmetric reverse of `build`. Like `build` and unlike the implicit teardown
/// during `uninstall`, this is an explicit user action: it is not gated by
/// `allow_app_hooks` and a nonzero exit propagates as a real error.
pub async fn handle_unbuild(config: &Config, app_id: &str) -> Result<()> {
    let categories = metadata::load_active_categories(config, Some(app_id)).await?;
    let cat = categories
        .iter()
        .find(|c| c.name == app_id)
        .ok_or_else(|| anyhow::anyhow!("app preset category not found: {app_id}"))?;

    let Some((teardown, runtime)) = cat
        .artifact
        .as_ref()
        .and_then(|a| a.teardown.as_deref().map(|t| (t, a.runtime)))
    else {
        bail!("app '{app_id}' does not define an artifact teardown script");
    };

    let mut command = artifact_command(config, app_id, teardown, runtime).await?;
    run_artifact_command(&mut command, app_id).await
}

/// Best-effort teardown run during `shine app uninstall`. Returns immediately
/// when the category declares no teardown. Unlike the explicit `unbuild`
/// command it is *implicit*, so — like `post_upgrade`/`post_install` hooks — it
/// is gated by `allow_app_hooks` for external presets and its failures are
/// non-fatal (a broken teardown must not block file removal). `dry_run` prints
/// the intended script without running it.
pub(crate) async fn run_teardown_for_uninstall(config: &Config, cat: &AppCategory, dry_run: bool) {
    let Some((teardown, runtime)) = cat
        .artifact
        .as_ref()
        .and_then(|a| a.teardown.as_deref().map(|t| (t, a.runtime)))
    else {
        return;
    };
    let app_id = &cat.name;

    if config.is_external_presets && !config.allow_app_hooks {
        println!(
            "  {} {app_id}: artifact teardown skipped (set allow_app_hooks = true to allow external app hooks; manual: shine app artifact remove {app_id})",
            colors::symbol("!"),
        );
        return;
    }

    if dry_run {
        println!(
            "  {} {app_id}: [dry-run] would run artifact teardown ({teardown})",
            colors::symbol("!"),
        );
        return;
    }

    let mut command = match artifact_command(config, app_id, teardown, runtime).await {
        Ok(command) => command,
        Err(e) => {
            eprintln!(
                "  {} {app_id}: artifact teardown skipped: {e:#}",
                colors::symbol("!"),
            );
            return;
        }
    };
    match command.status().await {
        Ok(status) if status.success() => {
            println!(
                "  {} {app_id}: artifact teardown completed",
                colors::symbol("✓")
            );
        }
        Ok(status) => {
            eprintln!(
                "  {} {app_id}: artifact teardown failed: exited with {status}",
                colors::symbol("!"),
            );
        }
        Err(e) => {
            eprintln!(
                "  {} {app_id}: artifact teardown failed: {e}",
                colors::symbol("!"),
            );
        }
    }
}

/// Resolves an artifact script (overlay copy wins over the source copy) and
/// builds a `Command` carrying the full `SHINE_APP_*` env contract plus the
/// active `[env]` table. Shared by `build` (`script`) and the teardown paths
/// (`teardown`) so both get identical inputs.
async fn artifact_command(
    config: &Config,
    app_id: &str,
    script_name: &str,
    runtime: metadata::ArtifactRuntime,
) -> Result<Command> {
    if !config.is_external_presets {
        crate::presets::extract_prefix(&format!("app/{app_id}"), config.presets_dir(), true)
            .await?;
    }
    let source_dir = config.presets_dir().join("app").join(app_id);

    let overlay_dir = config
        .active_presets_overlay_dir()
        .map(|dir| dir.join("app").join(app_id))
        .filter(|dir| dir.exists());

    let (resolved_app_dir, script_path) = if let Some(overlay_dir) = &overlay_dir
        && overlay_dir.join(script_name).exists()
    {
        (overlay_dir.clone(), overlay_dir.join(script_name))
    } else {
        let candidate = source_dir.join(script_name);
        if !candidate.exists() {
            bail!("app '{app_id}' artifact script not found: {script_name}");
        }
        (source_dir.clone(), candidate)
    };

    let http_dir = config.shine_dir().join("http").join("app").join(app_id);
    let cache_dir = BaseDirs::new()
        .context("resolving system cache directory")?
        .cache_dir()
        .join("shine")
        .join("app")
        .join(app_id);
    let state_dir = config.shine_dir().join("state").join("app").join(app_id);
    for dir in [&http_dir, &cache_dir, &state_dir] {
        fs::create_dir_all(dir)
            .await
            .with_context(|| format!("creating directory: {}", dir.display()))?;
    }

    // Inject the active `[env]` table so a build/teardown script can read
    // user-configured values like `SURGE_PROFILE`. Values are passed as stored
    // (no decryption) — the same as the `template` transform — so building never
    // triggers a secret decryption prompt (e.g. Touch ID / GPG) for unrelated
    // `_SECRET` keys. The `SHINE_APP_*` contract vars are set afterwards so they
    // win on any (unexpected) name collision with a user `[env]` key.
    let env_config = crate::env::EnvConfig::load_or_init(config).await?;

    let mut command = match runtime {
        metadata::ArtifactRuntime::Bun => {
            // Cross-platform: run the script via `bun <script>` (like shine's bun
            // shell presets). bun is an external prerequisite — fail clearly if
            // it is missing rather than emitting a raw spawn error.
            crate::proc::ensure_command("bun").with_context(|| {
                format!("app '{app_id}' artifact requires Bun (https://bun.sh)")
            })?;
            let mut command = Command::new("bun");
            command.arg(&script_path);
            command
        }
        metadata::ArtifactRuntime::Native => Command::new(&script_path),
    };
    command
        .current_dir(&resolved_app_dir)
        .envs(env_config.as_map())
        .env("SHINE_APP_ID", app_id)
        .env("SHINE_APP_DIR", &resolved_app_dir)
        .env("SHINE_APP_SOURCE_DIR", &source_dir)
        .env("SHINE_APP_HTTP_DIR", &http_dir)
        .env("SHINE_CONFIG_DIR", config.shine_dir())
        .env("SHINE_CACHE_DIR", &cache_dir)
        .env("SHINE_STATE_DIR", &state_dir);
    if let Some(overlay_dir) = &overlay_dir {
        command.env("SHINE_APP_OVERLAY_DIR", overlay_dir);
    }

    Ok(command)
}

/// Runs a prepared artifact `Command` with inherited (live) stdio and turns a
/// nonzero exit into a real error — the explicit-command semantics shared by
/// `build` and `unbuild`.
async fn run_artifact_command(command: &mut Command, app_id: &str) -> Result<()> {
    let status = command
        .status()
        .await
        .with_context(|| format!("running artifact script for '{app_id}'"))?;
    if !status.success() {
        bail!("artifact script for '{app_id}' exited with {status}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use tokio::fs;

    async fn make_temp_dir() -> PathBuf {
        crate::test_support::make_temp_dir("shine-apps-build").await
    }

    async fn write_sample_category(dir: &Path, script_body: &str) {
        let cat_dir = dir.join("presets/app/sample");
        fs::create_dir_all(&cat_dir).await.unwrap();
        fs::write(
            cat_dir.join("shine.toml"),
            "description = \"Sample app\"\ndest = \"~/.config/sample\"\n\n[artifact]\nscript = \"build.sh\"\n\n[[files]]\nsource = \"config.toml\"\n",
        )
        .await
        .unwrap();
        fs::write(cat_dir.join("config.toml"), b"name = \"sample\"\n")
            .await
            .unwrap();
        let script_path = cat_dir.join("build.sh");
        fs::write(&script_path, script_body).await.unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&script_path).await.unwrap().permissions();
            perms.set_mode(perms.mode() | 0o111);
            fs::set_permissions(&script_path, perms).await.unwrap();
        }
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn build_bails_when_no_artifact_declared() {
        let dir = make_temp_dir().await;
        let cat_dir = dir.join("presets/app/sample");
        fs::create_dir_all(&cat_dir).await.unwrap();
        fs::write(
            cat_dir.join("shine.toml"),
            "description = \"Sample app\"\ndest = \"~/.config/sample\"\n\n[[files]]\nsource = \"config.toml\"\n",
        )
        .await
        .unwrap();
        fs::write(cat_dir.join("config.toml"), b"name = \"sample\"\n")
            .await
            .unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        let err = handle_build(&config, "sample").await.unwrap_err();
        assert!(
            err.to_string()
                .contains("does not define an artifact script")
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn build_bails_for_unknown_app_id() {
        let dir = make_temp_dir().await;
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        let err = handle_build(&config, "doesnotexist").await.unwrap_err();
        assert!(err.to_string().contains("app preset category not found"));

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn build_runs_script_with_contract_env_vars_and_working_directory() {
        let dir = make_temp_dir().await;
        let marker = dir.join("marker.txt");
        write_sample_category(
            &dir,
            &format!(
                "#!/bin/sh\nset -e\npwd > \"{marker}\"\necho \"$SHINE_APP_ID\" >> \"{marker}\"\necho \"$SHINE_APP_HTTP_DIR\" >> \"{marker}\"\ntest -d \"$SHINE_CACHE_DIR\"\ntest -d \"$SHINE_STATE_DIR\"\n",
                marker = marker.display()
            ),
        )
        .await;

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        handle_build(&config, "sample").await.unwrap();

        let content = fs::read_to_string(&marker).await.unwrap();
        let mut lines = content.lines();
        let expected_app_dir = std::fs::canonicalize(dir.join("presets/app/sample")).unwrap();
        assert_eq!(
            lines.next().unwrap(),
            expected_app_dir.display().to_string()
        );
        assert_eq!(lines.next().unwrap(), "sample");
        assert_eq!(
            lines.next().unwrap(),
            config
                .shine_dir()
                .join("http")
                .join("app")
                .join("sample")
                .display()
                .to_string()
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn build_injects_env_table_into_script() {
        let dir = make_temp_dir().await;
        let marker = dir.join("env-marker.txt");
        write_sample_category(
            &dir,
            &format!(
                "#!/bin/sh\nset -e\nprintf '%s' \"$SURGE_PROFILE\" > \"{marker}\"\n",
                marker = marker.display()
            ),
        )
        .await;

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        config
            .env
            .insert("SURGE_PROFILE".into(), "/abs/path/Profile.conf".into());
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        handle_build(&config, "sample").await.unwrap();

        assert_eq!(
            fs::read_to_string(&marker).await.unwrap(),
            "/abs/path/Profile.conf"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn build_prefers_overlay_script_over_source_script() {
        let dir = make_temp_dir().await;
        write_sample_category(&dir, "#!/bin/sh\nexit 1\n").await;

        let overlay_dir = dir.join("overlay");
        let overlay_cat_dir = overlay_dir.join("app/sample");
        fs::create_dir_all(&overlay_cat_dir).await.unwrap();
        let marker = dir.join("overlay-ran");
        let overlay_script = overlay_cat_dir.join("build.sh");
        fs::write(
            &overlay_script,
            format!("#!/bin/sh\ntouch \"{}\"\n", marker.display()),
        )
        .await
        .unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&overlay_script).await.unwrap().permissions();
            perms.set_mode(perms.mode() | 0o111);
            fs::set_permissions(&overlay_script, perms).await.unwrap();
        }

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        config.presets_overlay_dir_override = Some(overlay_dir);
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        handle_build(&config, "sample").await.unwrap();

        assert!(
            marker.exists(),
            "overlay build.sh should run instead of the source one"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn build_falls_back_to_source_script_when_overlay_has_only_content() {
        let dir = make_temp_dir().await;
        let marker = dir.join("source-ran");
        write_sample_category(
            &dir,
            &format!("#!/bin/sh\ntouch \"{}\"\n", marker.display()),
        )
        .await;

        let overlay_dir = dir.join("overlay");
        let overlay_cat_dir = overlay_dir.join("app/sample");
        fs::create_dir_all(&overlay_cat_dir).await.unwrap();
        fs::write(overlay_cat_dir.join("config.toml"), "name = \"overlay\"\n")
            .await
            .unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        config.presets_overlay_dir_override = Some(overlay_dir);
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        handle_build(&config, "sample").await.unwrap();

        assert!(
            marker.exists(),
            "source build script should run when the overlay has no artifact script"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn build_propagates_nonzero_script_exit_as_error() {
        let dir = make_temp_dir().await;
        write_sample_category(&dir, "#!/bin/sh\nexit 7\n").await;

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        let err = handle_build(&config, "sample").await.unwrap_err();
        assert!(err.to_string().contains("exited with"));

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    async fn write_teardown_category(dir: &Path, teardown_body: &str) {
        let cat_dir = dir.join("presets/app/sample");
        fs::create_dir_all(&cat_dir).await.unwrap();
        fs::write(
            cat_dir.join("shine.toml"),
            "description = \"Sample app\"\ndest = \"~/.config/sample\"\n\n[artifact]\nscript = \"build.sh\"\nteardown = \"unbuild.sh\"\n\n[[files]]\nsource = \"config.toml\"\n",
        )
        .await
        .unwrap();
        fs::write(cat_dir.join("config.toml"), b"name = \"sample\"\n")
            .await
            .unwrap();
        fs::write(cat_dir.join("build.sh"), "#!/bin/sh\nexit 0\n")
            .await
            .unwrap();
        let script_path = cat_dir.join("unbuild.sh");
        fs::write(&script_path, teardown_body).await.unwrap();
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script_path).await.unwrap().permissions();
        perms.set_mode(perms.mode() | 0o111);
        fs::set_permissions(&script_path, perms).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn unbuild_runs_teardown_script() {
        let dir = make_temp_dir().await;
        let marker = dir.join("unbuild-ran");
        write_teardown_category(
            &dir,
            &format!("#!/bin/sh\ntouch \"{}\"\n", marker.display()),
        )
        .await;

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        handle_unbuild(&config, "sample").await.unwrap();
        assert!(marker.exists(), "teardown script should have run");

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn unbuild_bails_when_no_teardown_declared() {
        let dir = make_temp_dir().await;
        write_sample_category(&dir, "#!/bin/sh\nexit 0\n").await;

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        let err = handle_unbuild(&config, "sample").await.unwrap_err();
        assert!(
            err.to_string()
                .contains("does not define an artifact teardown script")
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn unbuild_propagates_nonzero_teardown_exit() {
        let dir = make_temp_dir().await;
        write_teardown_category(&dir, "#!/bin/sh\nexit 5\n").await;

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        let err = handle_unbuild(&config, "sample").await.unwrap_err();
        assert!(err.to_string().contains("exited with"));

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn teardown_for_uninstall_is_gated_for_external_presets() {
        let dir = make_temp_dir().await;
        let marker = dir.join("teardown-ran");
        write_teardown_category(
            &dir,
            &format!("#!/bin/sh\ntouch \"{}\"\n", marker.display()),
        )
        .await;

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        let categories = metadata::load_active_categories(&config, Some("sample"))
            .await
            .unwrap();
        let cat = categories.iter().find(|c| c.name == "sample").unwrap();

        // External preset without the opt-in: teardown must be skipped.
        run_teardown_for_uninstall(&config, cat, false).await;
        assert!(!marker.exists(), "external teardown must be gated");

        // Opt in: teardown runs.
        config.allow_app_hooks = true;
        run_teardown_for_uninstall(&config, cat, false).await;
        assert!(marker.exists(), "teardown should run once opted in");

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn teardown_for_uninstall_dry_run_does_not_execute() {
        let dir = make_temp_dir().await;
        let marker = dir.join("teardown-ran");
        write_teardown_category(
            &dir,
            &format!("#!/bin/sh\ntouch \"{}\"\n", marker.display()),
        )
        .await;

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        config.allow_app_hooks = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        let categories = metadata::load_active_categories(&config, Some("sample"))
            .await
            .unwrap();
        let cat = categories.iter().find(|c| c.name == "sample").unwrap();

        run_teardown_for_uninstall(&config, cat, true).await;
        assert!(!marker.exists(), "dry-run teardown must not execute");

        fs::remove_dir_all(&dir).await.unwrap();
    }
}
