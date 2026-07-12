use super::metadata;
use crate::config::Config;
use anyhow::{Context, Result, bail};
use directories::BaseDirs;
use tokio::fs;
use tokio::process::Command;

/// Runs the `[artifact].script` declared by an app preset (`shine app build <app-id>`).
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
        && overlay_dir.join(&artifact.script).exists()
    {
        (overlay_dir.clone(), overlay_dir.join(&artifact.script))
    } else {
        let candidate = source_dir.join(&artifact.script);
        if !candidate.exists() {
            bail!(
                "app '{app_id}' artifact script not found: {}",
                artifact.script
            );
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

    // Inject the active `[env]` table so a build script can read user-configured
    // values like `SURGE_PROFILE`. Values are passed as stored (no decryption) —
    // the same as the `template` transform — so building never triggers a secret
    // decryption prompt (e.g. Touch ID / GPG) for unrelated `_SECRET` keys. The
    // `SHINE_APP_*` contract vars are set afterwards so they win on any
    // (unexpected) name collision with a user `[env]` key.
    let env_config = crate::env::EnvConfig::load_or_init(config).await?;

    let mut command = Command::new(&script_path);
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

    let status = command
        .status()
        .await
        .with_context(|| format!("running artifact script: {}", script_path.display()))?;
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
}
