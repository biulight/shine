//! Dotenv for workspace environments.
use super::WORKSPACE_FORMAT_VERSION;
use super::{
    BTreeMap, BTreeSet, Config, Context, DocumentMut, ENV_SOURCE_FORMAT_VERSION,
    EnvWorkspaceExportFormat, Path, PathBuf, Result, WORKSPACE_FILE, Zeroizing,
    absolute_from_current, atomic_write, atomic_write_new, atomic_write_private,
    atomic_write_private_new, bail, compile_export_sources, find_workspace_optional,
    load_workspace, local_cache, resolve_sources, validate_mode, value,
};

/// Initialize a workspace by copying conventional dotenv sources into Shine's
/// explicit TOML source format. The original dotenv files are never modified.
pub async fn handle_init_from_dotenv(
    from_dotenv: bool,
    requested_modes: &[String],
    secrets: &[String],
    force: bool,
    dry_run: bool,
) -> Result<()> {
    if !from_dotenv {
        bail!("pass --from-dotenv to initialize from conventional dotenv files");
    }

    let root = std::env::current_dir().context("resolving current directory")?;
    init_from_dotenv_at(&root, requested_modes, secrets, force, dry_run).await
}

#[allow(clippy::too_many_arguments)]
pub async fn handle_export(
    config: &Config,
    format: EnvWorkspaceExportFormat,
    workspace_arg: Option<&Path>,
    mode: &str,
    output: &Path,
    include_secrets: bool,
    force: bool,
    dry_run: bool,
) -> Result<()> {
    if format != EnvWorkspaceExportFormat::Dotenv {
        bail!("files export requires the file-group handler");
    }
    validate_mode(mode)?;
    let workspace_path = find_workspace_optional(workspace_arg)
        .await?
        .context("shine.workspace.toml was not found; pass --workspace")?;
    let local_config = local_cache::load_config(config, &workspace_path).await?;
    let config = &local_config;
    let workspace = load_workspace(&workspace_path).await?;
    let sources = resolve_sources(&workspace_path, &workspace.env.files, mode)?;
    let output = absolute_from_current(output)?;
    if !force {
        match tokio::fs::symlink_metadata(&output).await {
            Ok(_) => bail!(
                "{:?} already exists; rerun with --force to replace it",
                output.to_string_lossy()
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("inspecting export destination"),
        }
    }

    let values = compile_export_sources(&sources, config, include_secrets && !dry_run).await?;
    let contents = render_dotenv(&values)?;
    if dry_run {
        println!(
            "Would export {} non-secret variable(s) for mode {mode} to {:?}. Secrets were not decrypted or counted{}",
            values.len(),
            output.to_string_lossy(),
            if include_secrets {
                "; --include-secrets will be applied during actual export."
            } else {
                "."
            }
        );
    } else {
        let write_result = match (force, include_secrets) {
            (false, true) => atomic_write_private_new(&output, contents.as_bytes()).await,
            (false, false) => atomic_write_new(&output, contents.as_bytes()).await,
            (true, true) => atomic_write_private(&output, contents.as_bytes()).await,
            (true, false) => atomic_write(&output, contents.as_bytes()).await,
        };
        if let Err(error) = write_result {
            if !force
                && error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|error| error.kind() == std::io::ErrorKind::AlreadyExists)
            {
                bail!(
                    "{:?} already exists; rerun with --force to replace it",
                    output.to_string_lossy()
                );
            }
            return Err(error);
        }
        println!(
            "Exported {} variables for mode {mode} to {:?}{}",
            values.len(),
            output.to_string_lossy(),
            if include_secrets {
                " (including secrets)"
            } else {
                ""
            }
        );
        if include_secrets {
            eprintln!("Warning: the exported file contains plaintext secrets; do not commit it.");
        }
    }
    Ok(())
}

pub(super) fn render_dotenv(values: &BTreeMap<String, String>) -> Result<Zeroizing<String>> {
    let mut rendered = Zeroizing::new(String::new());
    for (key, value) in values {
        if value.contains('\0') {
            bail!("{key} contains a NUL byte and cannot be represented in dotenv format");
        }
        // Dotenv readers disagree on escapes and expand dollars even inside quotes
        // (notably Bun/Vite). Fail before publication instead of changing a value.
        if value.contains(['$', '\r']) {
            bail!(
                "{key} contains a dollar sign or carriage return that cannot be exported portably to dotenv; use `shine env run` for this value"
            );
        }
        let quote = if !value.contains(['"', '\\']) {
            '"'
        } else if !value.contains('\'') {
            '\''
        } else if !value.contains('`') {
            '`'
        } else {
            bail!("{key} has no safe dotenv quote delimiter; use `shine env run` for this value");
        };
        rendered.push_str(key);
        rendered.push('=');
        rendered.push(quote);
        rendered.push_str(value);
        rendered.push(quote);
        rendered.push('\n');
    }
    Ok(rendered)
}

pub(super) async fn init_from_dotenv_at(
    root: &Path,
    requested_modes: &[String],
    secrets: &[String],
    force: bool,
    dry_run: bool,
) -> Result<()> {
    let modes = dotenv_modes(root, requested_modes)?;
    let sources = dotenv_sources(root, &modes);
    let mut planned = Vec::new();
    let requested_secrets: BTreeSet<_> = secrets.iter().cloned().collect();
    for key in &requested_secrets {
        crate::env::validate_env_key(key)?;
    }
    let mut seen_keys = BTreeSet::new();

    for (input, output) in sources {
        if !input.is_file() {
            continue;
        }
        let contents = tokio::fs::read_to_string(&input)
            .await
            .with_context(|| format!("reading {}", input.display()))?;
        let values = parse_dotenv(&input, &contents)?;
        seen_keys.extend(values.keys().cloned());
        planned.push((output, render_source(&input, &values, &requested_secrets)));
    }
    if planned.is_empty() {
        bail!("no dotenv files found; expected .env, .env.local, or .env.<mode>");
    }
    for key in &requested_secrets {
        if !seen_keys.contains(key) {
            bail!("--secret {key} was not found in an imported dotenv file");
        }
    }

    let workspace = root.join(WORKSPACE_FILE);
    planned.push((workspace, render_workspace(&modes)));
    for (path, _) in &planned {
        if !force {
            match tokio::fs::symlink_metadata(path).await {
                Ok(_) => bail!(
                    "{} already exists; rerun with --force to replace generated files",
                    path.display()
                ),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error).context("inspecting import destination"),
            }
        }
    }

    for (path, contents) in &planned {
        let display = path.strip_prefix(root).unwrap_or(path).display();
        if dry_run {
            println!(
                "Would {} {display}",
                if force {
                    "write (replace if present)"
                } else {
                    "create"
                }
            );
        } else {
            let result = if force {
                atomic_write_private(path, contents.as_bytes()).await
            } else {
                atomic_write_private_new(path, contents.as_bytes()).await
            };
            if let Err(error) = result {
                if !force
                    && error
                        .downcast_ref::<std::io::Error>()
                        .is_some_and(|error| error.kind() == std::io::ErrorKind::AlreadyExists)
                {
                    bail!(
                        "{} already exists; rerun with --force to replace generated files",
                        path.display()
                    );
                }
                return Err(error);
            }
            println!(
                "{} {display}",
                if force {
                    "Wrote (replacing if present)"
                } else {
                    "Created"
                }
            );
        }
    }
    if !requested_secrets.is_empty() {
        println!("Run `shine env secret seal` after configuring an encryption recipient.");
    }
    Ok(())
}

pub(super) fn dotenv_modes(root: &Path, requested: &[String]) -> Result<Vec<String>> {
    let mut modes: BTreeSet<String> = requested.iter().cloned().collect();
    for mode in &modes {
        validate_mode(mode)?;
    }
    if modes.is_empty() {
        for entry in
            std::fs::read_dir(root).with_context(|| format!("reading {}", root.display()))?
        {
            let name = entry?.file_name();
            let name = name.to_string_lossy();
            let Some(suffix) = name.strip_prefix(".env.") else {
                continue;
            };
            if suffix == "local" || suffix.ends_with(".shine.toml") {
                continue;
            }
            let mode = suffix.strip_suffix(".local").unwrap_or(suffix);
            if mode.is_empty() || mode.contains('.') {
                continue;
            }
            validate_mode(mode)?;
            modes.insert(mode.to_owned());
        }
    }
    if modes.is_empty() {
        modes.insert("development".to_owned());
    }
    Ok(modes.into_iter().collect())
}

pub(super) fn dotenv_sources(root: &Path, modes: &[String]) -> Vec<(PathBuf, PathBuf)> {
    let mut files = vec![
        (root.join(".env"), root.join(".env.shine.toml")),
        (root.join(".env.local"), root.join(".env.local.shine.toml")),
    ];
    for mode in modes {
        files.push((
            root.join(format!(".env.{mode}")),
            root.join(format!(".env.{mode}.shine.toml")),
        ));
        files.push((
            root.join(format!(".env.{mode}.local")),
            root.join(format!(".env.{mode}.local.shine.toml")),
        ));
    }
    files
}

pub(super) fn render_workspace(modes: &[String]) -> String {
    let default_mode = &modes[0];
    let rendered_modes = modes
        .iter()
        .map(|mode| format!("\"{mode}\""))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "# Managed by `shine env workspace init --from-dotenv`.\n\
         # Edit the source files below; later files override earlier ones.\n\
         version = {WORKSPACE_FORMAT_VERSION}\n\n\
         [env]\n\
         # Run with: shine env run --mode {default_mode} -- <command>\n\
         modes = [{modes}]\n\
         default_mode = \"{default_mode}\"\n\
         files = [\n\
           \".env.shine.toml\", # shared defaults\n\
           \".env.local.shine.toml\", # local-only overrides; do not commit\n\
           \".env.{{mode}}.shine.toml\", # mode-specific values\n\
           \".env.{{mode}}.local.shine.toml\", # local mode overrides; do not commit\n\
         ]\n\n\
         # Add GPG recipients before sealing values in [secret].\n\
         # [env.encryption]\n\
         # gpg_recipients = [\"alice@example.com\", \"bob@example.com\"]\n",
        modes = rendered_modes,
    )
}

pub(super) fn render_source(
    input: &Path,
    values: &BTreeMap<String, String>,
    secrets: &BTreeSet<String>,
) -> String {
    let mut document = DocumentMut::new();
    document["version"] = value(ENV_SOURCE_FORMAT_VERSION as i64);
    let mut plain = toml_edit::Table::new();
    let mut secret = toml_edit::Table::new();
    for (key, value_text) in values {
        if secrets.contains(key) {
            secret[key] = value(value_text);
        } else {
            plain[key] = value(value_text);
        }
    }
    if !plain.is_empty() {
        document["plain"] = toml_edit::Item::Table(plain);
    }
    if !secret.is_empty() {
        document["secret"] = toml_edit::Item::Table(secret);
    }
    let source_name = input
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("dotenv file");
    let mut contents = format!(
        "# Imported from {source_name}. Keep non-sensitive values in [plain].\n\
         # Move sensitive values to [secret], then run `shine env secret seal`.\n"
    );
    contents.push_str(&document.to_string());
    if secrets.is_empty() {
        contents.push_str(
            "\n# Optional: move sensitive values here, then run `shine env secret seal`.\n[secret]\n",
        );
    }
    contents
}

pub(super) fn parse_dotenv(path: &Path, contents: &str) -> Result<BTreeMap<String, String>> {
    let mut values = BTreeMap::new();
    for (index, line) in contents.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line).trim_start();
        let Some((key, raw_value)) = line.split_once('=') else {
            bail!(
                "{}:{} is not a KEY=VALUE dotenv entry",
                path.display(),
                index + 1
            );
        };
        let key = key.trim();
        crate::env::validate_env_key(key)
            .with_context(|| format!("{}:{}", path.display(), index + 1))?;
        let value_text = parse_dotenv_value(path, index + 1, raw_value)?;
        values.insert(key.to_owned(), value_text);
    }
    Ok(values)
}

pub(super) fn parse_dotenv_value(path: &Path, line: usize, raw: &str) -> Result<String> {
    let raw = raw.trim();
    let value = if let Some(value) = raw.strip_prefix('\'') {
        parse_quoted_dotenv_value(value, '\'', "single")?
    } else if let Some(value) = raw.strip_prefix('"') {
        let value = parse_quoted_dotenv_value(value, '"', "double")?;
        if value.contains('\\') {
            bail!(
                "{}:{line} uses escaped double-quoted dotenv content; resolve it before importing",
                path.display()
            );
        }
        value
    } else if let Some(value) = raw.strip_prefix('`') {
        parse_quoted_dotenv_value(value, '`', "backtick")?
    } else {
        raw.split_once('#')
            .map(|(value, _)| value)
            .unwrap_or(raw)
            .trim_end()
    };
    if value.contains('$') {
        bail!(
            "{}:{line} uses dotenv interpolation or dollar syntax; resolve it before importing",
            path.display()
        );
    }
    Ok(value.to_owned())
}

pub(super) fn parse_quoted_dotenv_value<'a>(
    raw: &'a str,
    quote: char,
    style: &str,
) -> Result<&'a str> {
    let closing = raw
        .find(quote)
        .with_context(|| format!("unterminated {style}-quoted dotenv value"))?;
    let trailing = raw[closing + quote.len_utf8()..].trim_start();
    if !trailing.is_empty() && !trailing.starts_with('#') {
        bail!("unexpected content after {style}-quoted dotenv value");
    }
    Ok(&raw[..closing])
}
