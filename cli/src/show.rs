use crate::apps::{
    AppCategory, AppFile, AppManifest, installed_content_hash, load_embedded_categories,
    load_installed_categories, resolve_install_destination, source_bytes_for_file,
    source_hash_for_file,
};
use crate::check::FileStatus;
use crate::colors;
use crate::config::Config;
use crate::env::EnvConfig;
use crate::path_display;
use crate::shells::metadata::load_installed_categories as load_installed_shells;
use crate::shells::metadata::{ShellCategory, load_embedded_categories as load_embedded_shells};
use anyhow::{Context, Result, bail};
use similar::TextDiff;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Eq, PartialEq)]
enum ShowRef {
    AppCategory(String),
    AppFile { category: String, source: PathBuf },
    ShellCategory(String),
    ShellFile { category: String, command: String },
}

#[derive(Clone, Debug)]
struct TargetCandidate {
    canonical: String,
    aliases: BTreeSet<String>,
    item: ShowRef,
}

#[derive(Clone)]
struct AppShowFile {
    category: AppCategory,
    file: AppFile,
    destination: PathBuf,
    status: FileStatus,
    manifest_entry: Option<crate::apps::AppEntry>,
}

#[derive(Clone)]
struct ShellShowFile {
    category: ShellCategory,
    file: crate::shells::metadata::ShellFile,
    source_path: PathBuf,
    rendered_path: PathBuf,
    link_path: PathBuf,
    link_target: Option<PathBuf>,
    status: &'static str,
}

pub(crate) async fn handle_show(
    config: &Config,
    target: &str,
    diff: bool,
    verbose: bool,
) -> Result<()> {
    crate::config::print_presets_note(config);
    let app_files = collect_app_files(config).await?;
    let shell_files = collect_shell_files(config).await?;

    if app_files.is_empty() && shell_files.is_empty() {
        bail!("nothing installed yet. Run `shine shell install` or `shine app install`.");
    }

    let candidates = build_candidates(&app_files, &shell_files);
    let refs = resolve_target(target, &candidates)?;

    let mut first = true;
    for item in refs {
        if !first {
            println!();
        }
        first = false;
        match item {
            ShowRef::AppCategory(category) => {
                let mut files: Vec<_> = app_files
                    .iter()
                    .filter(|f| f.category.name == category)
                    .cloned()
                    .collect();
                files.sort_by_key(|f| f.file.source_rel.clone());
                for (index, file) in files.iter().enumerate() {
                    if index > 0 {
                        println!();
                    }
                    print_app_file(config, file, diff, verbose).await?;
                }
            }
            ShowRef::AppFile { category, source } => {
                let file = app_files
                    .iter()
                    .find(|f| f.category.name == category && f.file.source_rel == source)
                    .ok_or_else(|| anyhow::anyhow!("installed app config not found"))?;
                print_app_file(config, file, diff, verbose).await?;
            }
            ShowRef::ShellCategory(category) => {
                let mut files: Vec<_> = shell_files
                    .iter()
                    .filter(|f| f.category.name == category)
                    .cloned()
                    .collect();
                files.sort_by_key(|f| f.file.command_name.clone());
                for (index, file) in files.iter().enumerate() {
                    if index > 0 {
                        println!();
                    }
                    print_shell_file(config, file, diff, verbose).await?;
                }
            }
            ShowRef::ShellFile { category, command } => {
                let file = shell_files
                    .iter()
                    .find(|f| f.category.name == category && f.file.command_name == command)
                    .ok_or_else(|| anyhow::anyhow!("installed shell preset not found"))?;
                print_shell_file(config, file, diff, verbose).await?;
            }
        }
    }

    Ok(())
}

async fn collect_app_files(config: &Config) -> Result<Vec<AppShowFile>> {
    let categories = if config.is_external_presets {
        load_installed_categories(config, None).await?
    } else {
        load_embedded_categories(None)?
    };
    let manifest = AppManifest::load(config.shine_dir()).await?;
    let env = EnvConfig::load_or_init(config).await.ok();
    let empty_map = BTreeMap::new();
    let env_map = env.as_ref().map(|e| e.as_map()).unwrap_or(&empty_map);

    let mut files = Vec::new();
    for category in categories {
        for file in &category.files {
            let source_key = format!("app/{}/{}", category.name, file.source_rel.display());
            let destination = match resolve_install_destination(&category, file, config) {
                Ok(dest) => dest,
                Err(e) => {
                    if manifest
                        .entries
                        .iter()
                        .any(|entry| entry.source == source_key)
                    {
                        eprintln!(
                            "warning: skipping {}/{}: {e:#}",
                            category.name,
                            file.source_rel.display()
                        );
                    }
                    continue;
                }
            };
            let Some(entry) = manifest.find_by_dest(&destination).cloned() else {
                continue;
            };
            let status = app_file_status(config, &category, file, &entry, env_map).await;
            files.push(AppShowFile {
                category: category.clone(),
                file: file.clone(),
                destination,
                status,
                manifest_entry: Some(entry),
            });
        }
    }

    Ok(files)
}

async fn collect_shell_files(config: &Config) -> Result<Vec<ShellShowFile>> {
    let categories = if config.is_external_presets {
        load_installed_shells(config, None).await?
    } else {
        load_embedded_shells(None)?
    };

    let mut files = Vec::new();
    for category in categories {
        for file in &category.files {
            let source_path = config
                .presets_dir()
                .join("shell")
                .join(&category.name)
                .join(&file.source_rel);
            let rendered_path = config
                .rendered_dir()
                .join("shell")
                .join(&category.name)
                .join(&file.source_rel);
            let link_path = crate::bin_links::command_path_for_name(
                config.bin_dir(),
                std::ffi::OsStr::new(&file.command_name),
            );

            let source_exists = source_path.exists();
            let rendered_exists = rendered_path.exists();
            let (link_exists, link_target) = read_link_target(&link_path).await?;
            if !source_exists && !rendered_exists && !link_exists {
                continue;
            }

            let status = match (source_exists || rendered_exists, link_exists) {
                (true, true) => "up-to-date",
                (true, false) => "preset present, bin symlink missing",
                (false, true) => "bin symlink present, script missing",
                (false, false) => "not installed",
            };

            files.push(ShellShowFile {
                category: category.clone(),
                file: file.clone(),
                source_path,
                rendered_path,
                link_path,
                link_target,
                status,
            });
        }
    }
    Ok(files)
}

fn build_candidates(
    app_files: &[AppShowFile],
    shell_files: &[ShellShowFile],
) -> Vec<TargetCandidate> {
    let mut candidates = Vec::new();

    for category in app_files
        .iter()
        .map(|f| f.category.name.clone())
        .collect::<BTreeSet<_>>()
    {
        let mut aliases = BTreeSet::from([category.clone()]);
        aliases.insert(format!("app/{category}"));
        candidates.push(TargetCandidate {
            canonical: format!("app/{category}"),
            aliases,
            item: ShowRef::AppCategory(category),
        });
    }

    for file in app_files {
        let source = file.file.source_rel.display().to_string();
        let mut aliases =
            BTreeSet::from([format!("{}/{}", file.category.name, source), source.clone()]);
        if let Some(display_name) = &file.file.display_name {
            aliases.insert(display_name.clone());
        }
        if let Some(name) = file.destination.file_name().and_then(|n| n.to_str()) {
            aliases.insert(name.to_string());
        }
        candidates.push(TargetCandidate {
            canonical: format!("app/{}/{}", file.category.name, source),
            aliases,
            item: ShowRef::AppFile {
                category: file.category.name.clone(),
                source: file.file.source_rel.clone(),
            },
        });
    }

    for category in shell_files
        .iter()
        .map(|f| f.category.name.clone())
        .collect::<BTreeSet<_>>()
    {
        let mut aliases = BTreeSet::from([category.clone()]);
        aliases.insert(format!("shell/{category}"));
        candidates.push(TargetCandidate {
            canonical: format!("shell/{category}"),
            aliases,
            item: ShowRef::ShellCategory(category),
        });
    }

    for file in shell_files {
        let source = file.file.source_rel.display().to_string();
        let mut aliases = BTreeSet::from([
            file.file.command_name.clone(),
            source.clone(),
            format!("{}/{}", file.category.name, file.file.command_name),
        ]);
        if let Some(name) = file.file.source_rel.file_stem().and_then(|n| n.to_str()) {
            aliases.insert(name.to_string());
        }
        candidates.push(TargetCandidate {
            canonical: format!("shell/{}/{}", file.category.name, file.file.command_name),
            aliases,
            item: ShowRef::ShellFile {
                category: file.category.name.clone(),
                command: file.file.command_name.clone(),
            },
        });
    }

    candidates
}

fn resolve_target(target: &str, candidates: &[TargetCandidate]) -> Result<Vec<ShowRef>> {
    let trimmed = target.trim();
    if trimmed.is_empty() {
        bail!("info target must not be empty");
    }

    let exact: Vec<_> = candidates
        .iter()
        .filter(|c| c.canonical == trimmed)
        .collect();
    if exact.len() == 1 {
        return Ok(vec![exact[0].item.clone()]);
    }
    if exact.len() > 1 {
        return ambiguity(trimmed, exact);
    }

    let alias: Vec<_> = candidates
        .iter()
        .filter(|c| c.aliases.contains(trimmed))
        .collect();
    if alias.len() == 1 {
        return Ok(vec![alias[0].item.clone()]);
    }
    if alias.len() > 1 {
        return ambiguity(trimmed, alias);
    }

    bail!("{}", missing_target_message(trimmed, candidates));
}

fn ambiguity(target: &str, matches: Vec<&TargetCandidate>) -> Result<Vec<ShowRef>> {
    let choices = matches
        .iter()
        .map(|c| c.canonical.as_str())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join(", ");
    bail!("ambiguous info target '{target}'. Use one of: {choices}");
}

fn missing_target_message(target: &str, candidates: &[TargetCandidate]) -> String {
    let suggestions = suggested_targets(target, candidates);
    let mut message = format!("installed item not found: {target}");

    if suggestions.is_empty() {
        let available = grouped_available_targets(candidates);

        if !available.is_empty() {
            message.push_str("\n\nAvailable installed targets:");
            for (heading, targets) in available {
                message.push_str(&format!("\n  {heading}"));
                for target in targets {
                    message.push_str(&format!("\n    {target}"));
                }
            }
        }
    } else {
        message.push_str("\n\nDid you mean:");
        for target in suggestions {
            message.push_str(&format!("\n  {target}"));
        }
    }

    message.push_str("\n\nRun `shine list` to see installed configs.");
    message.push_str(
        "\nUse full targets like `app/docker-desktop/settings-store.jsonc` for exact file info.",
    );
    message
}

fn suggested_targets(target: &str, candidates: &[TargetCandidate]) -> Vec<String> {
    let needle = target.to_ascii_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }

    let matches = candidates
        .iter()
        .filter(|candidate| is_suggested_target(&needle, candidate))
        .collect::<Vec<_>>();
    let matched_parents = matches
        .iter()
        .filter_map(|candidate| match &candidate.item {
            ShowRef::AppCategory(category) => Some(("app", category.as_str())),
            ShowRef::ShellCategory(category) => Some(("shell", category.as_str())),
            ShowRef::AppFile { .. } | ShowRef::ShellFile { .. } => None,
        })
        .collect::<BTreeSet<_>>();

    matches
        .into_iter()
        .filter(|candidate| !has_matched_parent(candidate, &matched_parents))
        .map(display_target_name)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn grouped_available_targets(
    candidates: &[TargetCandidate],
) -> BTreeMap<&'static str, Vec<String>> {
    let mut groups: BTreeMap<&'static str, BTreeSet<String>> = BTreeMap::new();
    for candidate in candidates {
        match &candidate.item {
            ShowRef::AppCategory(category) => {
                groups
                    .entry("App Configs")
                    .or_default()
                    .insert(category.clone());
            }
            ShowRef::ShellCategory(category) => {
                groups
                    .entry("Shell Presets")
                    .or_default()
                    .insert(category.clone());
            }
            ShowRef::AppFile { .. } | ShowRef::ShellFile { .. } => {}
        }
    }

    groups
        .into_iter()
        .map(|(heading, targets)| (heading, targets.into_iter().collect()))
        .collect()
}

fn has_matched_parent(
    candidate: &TargetCandidate,
    matched_parents: &BTreeSet<(&str, &str)>,
) -> bool {
    match &candidate.item {
        ShowRef::AppFile { category, .. } => matched_parents.contains(&("app", category.as_str())),
        ShowRef::ShellFile { category, .. } => {
            matched_parents.contains(&("shell", category.as_str()))
        }
        ShowRef::AppCategory(_) | ShowRef::ShellCategory(_) => false,
    }
}

fn display_target_name(candidate: &TargetCandidate) -> String {
    match &candidate.item {
        ShowRef::AppCategory(category) | ShowRef::ShellCategory(category) => category.clone(),
        ShowRef::AppFile { category, source } => format!("{category}/{}", source.display()),
        ShowRef::ShellFile { category, command } => format!("{category}/{command}"),
    }
}

fn is_suggested_target(needle: &str, candidate: &TargetCandidate) -> bool {
    std::iter::once(candidate.canonical.as_str())
        .chain(candidate.aliases.iter().map(String::as_str))
        .any(|value| {
            let value = value.to_ascii_lowercase();
            value.contains(needle)
                || value
                    .split(['/', '-', '_', '.'])
                    .filter(|part| !part.is_empty())
                    .any(|part| part == needle || part.starts_with(needle))
        })
}

async fn print_app_file(
    config: &Config,
    item: &AppShowFile,
    diff: bool,
    verbose: bool,
) -> Result<()> {
    let label = fallback_app_label(&item.category, &item.file);
    let source_path = config
        .presets_dir()
        .join("app")
        .join(&item.category.name)
        .join(&item.file.source_rel);

    println!("{}", colors::bold(&format!("App Config: {label}")));
    if let Some(desc) = &item.file.description {
        println!("{}  {desc}", colors::dim("Description"));
    } else if let Some(desc) = &item.category.description {
        println!("{}  {desc}", colors::dim("Description"));
    }
    println!(
        "{}       {}",
        colors::dim("Source"),
        path_display::format_home(&source_path, &config.home_dir)
    );
    println!(
        "{}  {}",
        colors::dim("Destination"),
        path_display::format_home(&item.destination, &config.home_dir)
    );
    println!(
        "{}       {}",
        colors::dim("Status"),
        colored_app_status(item.status)
    );
    if !item.file.transforms.is_empty() {
        println!(
            "{}   {}",
            colors::dim("Transforms"),
            item.file.transforms.join(", ")
        );
    }
    if let Some(entry) = &item.manifest_entry {
        println!("{} {}", colors::dim("Manifest hash"), entry.content_hash);
        if let Some(backup) = &entry.backup {
            println!(
                "{}       {}",
                colors::dim("Backup"),
                path_display::format_home(backup, &config.home_dir)
            );
        }
    }
    if diff {
        let diff_output = app_diff_output(config, item).await?;
        print_block("Diff", &item.destination, &diff_output);
    }
    if verbose {
        print_file_content(&item.destination, "Content").await?;
    }
    Ok(())
}

async fn print_shell_file(
    config: &Config,
    item: &ShellShowFile,
    diff: bool,
    verbose: bool,
) -> Result<()> {
    let label = format!("{}/{}", item.category.name, item.file.command_name);
    println!("{}", colors::bold(&format!("Shell Preset: {label}")));
    if let Some(desc) = item.file.description.first() {
        println!("{}  {desc}", colors::dim("Description"));
    }
    println!(
        "{}       {}",
        colors::dim("Source"),
        path_display::format_home(&item.source_path, &config.home_dir)
    );
    println!(
        "{}     {}",
        colors::dim("Bin link"),
        path_display::format_home(&item.link_path, &config.home_dir)
    );
    if let Some(target) = &item.link_target {
        println!(
            "{} {}",
            colors::dim("Link target"),
            path_display::format_home(target, &config.home_dir)
        );
    }
    println!(
        "{}       {}",
        colors::dim("Status"),
        colored_shell_status(item.status)
    );
    println!("{} {}", colors::dim("Needs source"), item.file.needs_source);

    let content_path = item
        .link_target
        .as_ref()
        .filter(|target| target.starts_with(&item.rendered_path) || **target == item.rendered_path)
        .cloned()
        .or_else(|| {
            item.rendered_path
                .exists()
                .then(|| item.rendered_path.clone())
        })
        .unwrap_or_else(|| item.source_path.clone());

    if content_path == item.rendered_path {
        println!(
            "{}     {}",
            colors::dim("Rendered"),
            path_display::format_home(&item.rendered_path, &config.home_dir)
        );
    }
    if diff {
        let diff_output = shell_diff_output(config, item, &content_path).await?;
        print_block("Diff", &content_path, &diff_output);
    }
    if verbose {
        print_file_content(&content_path, "Content").await?;
    }
    Ok(())
}

fn print_heading(heading: &str, path: &Path) {
    println!();
    println!(
        "{}",
        colors::dim(&format!(
            "--- {heading}: {} ---",
            path_display::format(path)
        ))
    );
}

fn print_block(heading: &str, path: &Path, text: &str) {
    print_heading(heading, path);
    print!("{text}");
    if !text.ends_with('\n') {
        println!();
    }
}

async fn print_file_content(path: &Path, heading: &str) -> Result<()> {
    print_heading(heading, path);
    let bytes = tokio::fs::read(path)
        .await
        .with_context(|| format!("reading installed content: {}", path.display()))?;
    print!("{}", String::from_utf8_lossy(&bytes));
    if !bytes.ends_with(b"\n") {
        println!();
    }
    Ok(())
}

async fn read_link_target(path: &Path) -> Result<(bool, Option<PathBuf>)> {
    let is_link = tokio::fs::symlink_metadata(path)
        .await
        .map(|meta| meta.file_type().is_symlink())
        .unwrap_or(false);
    if !is_link {
        return Ok((path.exists(), None));
    }
    let target = tokio::fs::read_link(path)
        .await
        .with_context(|| format!("reading symlink: {}", path.display()))?;
    Ok((true, Some(target)))
}

fn fallback_app_label(category: &AppCategory, file: &AppFile) -> String {
    file.display_name
        .clone()
        .unwrap_or_else(|| format!("{}/{}", category.name, file.source_rel.display()))
}

async fn app_file_status(
    config: &Config,
    category: &AppCategory,
    file: &AppFile,
    entry: &crate::apps::AppEntry,
    env: &BTreeMap<String, String>,
) -> FileStatus {
    if !entry.destination.exists() {
        return FileStatus::Missing;
    }
    match tokio::fs::read(&entry.destination).await {
        Ok(bytes) => match installed_content_hash(file, &bytes) {
            Ok(Some(installed_hash)) if installed_hash == entry.content_hash => {
                match source_hash_for_file(config, category, file, env).await {
                    Some(source_hash) if source_hash != entry.content_hash => {
                        FileStatus::UpdateAvail
                    }
                    _ => FileStatus::UpToDate,
                }
            }
            Ok(None) => FileStatus::Missing,
            Ok(Some(_)) | Err(_) => FileStatus::UserModified,
        },
        Err(_) => FileStatus::Missing,
    }
}

fn status_parts(status: FileStatus) -> (&'static str, &'static str) {
    match status {
        FileStatus::Missing => ("destination missing", "!"),
        FileStatus::UserModified => ("user modified", "~"),
        FileStatus::UpdateAvail => ("update available", "↑"),
        FileStatus::Partial => ("partial install", "~"),
        FileStatus::UpToDate => ("up-to-date", "✓"),
        FileStatus::NotInstalled => ("not installed", "✗"),
    }
}

fn status_sym(status: FileStatus) -> &'static str {
    status_parts(status).1
}

fn colored_app_status(status: FileStatus) -> String {
    colors::status_label(status_parts(status).0, status_sym(status))
}

// status is a free-form &'static str produced by the shell subsystem; _ => "~" is intentional.
fn shell_status_sym(status: &str) -> &'static str {
    match status {
        "up-to-date" => "✓",
        "update available" => "↑",
        "preset present, bin symlink missing"
        | "bin symlink present, script missing"
        | "bin symlink present, preset missing" => "~",
        "rendered script missing" => "!",
        "not installed" => "✗",
        _ => "~",
    }
}

fn colored_shell_status(status: &str) -> String {
    colors::status_label(status, shell_status_sym(status))
}

async fn app_diff_output(config: &Config, item: &AppShowFile) -> Result<String> {
    let env = EnvConfig::load_or_init(config).await.ok();
    let empty_map = BTreeMap::new();
    let env_map = env.as_ref().map(|e| e.as_map()).unwrap_or(&empty_map);
    let expected = match source_bytes_for_file(config, &item.category, &item.file, env_map).await {
        Some(bytes) => bytes,
        None => {
            return Ok("Unable to render expected content from the active preset.\n".to_string());
        }
    };

    let current = match tokio::fs::read(&item.destination).await {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(format!(
                "Current file is missing: {}.\n",
                item.destination.display()
            ));
        }
        Err(err) => {
            return Ok(format!(
                "Unable to read current file {}: {err}\n",
                item.destination.display()
            ));
        }
    };

    Ok(render_diff_or_note(
        &current,
        &expected,
        &item.destination.to_string_lossy(),
        &format!(
            "expected: app/{}/{}",
            item.category.name,
            item.file.source_rel.display()
        ),
    ))
}

async fn shell_diff_output(
    config: &Config,
    item: &ShellShowFile,
    current_path: &Path,
) -> Result<String> {
    let expected = match shell_expected_bytes(config, item).await? {
        Some(bytes) => bytes,
        None => {
            return Ok(
                "Unable to render expected shell content from the active preset.\n".to_string(),
            );
        }
    };

    let current = match tokio::fs::read(current_path).await {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(format!(
                "Current script is missing: {}.\n",
                current_path.display()
            ));
        }
        Err(err) => {
            return Ok(format!(
                "Unable to read current script {}: {err}\n",
                current_path.display()
            ));
        }
    };

    Ok(render_diff_or_note(
        &current,
        &expected,
        &current_path.to_string_lossy(),
        &format!(
            "expected: shell/{}/{}",
            item.category.name,
            item.file.source_rel.display()
        ),
    ))
}

async fn shell_expected_bytes(config: &Config, item: &ShellShowFile) -> Result<Option<Vec<u8>>> {
    let source = match tokio::fs::read(&item.source_path).await {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(err).with_context(|| {
                format!(
                    "reading shell preset source: {}",
                    item.source_path.display()
                )
            });
        }
    };

    if !crate::presets::parse_template_annotation(&source) {
        return Ok(Some(source));
    }

    let env = EnvConfig::load_or_init(config).await?;
    let rendered = crate::apps::apply_transforms(&["template".to_string()], &source, env.as_map())
        .with_context(|| format!("rendering shell template: {}", item.source_path.display()))?;
    Ok(Some(rendered))
}

fn render_diff_or_note(
    current: &[u8],
    expected: &[u8],
    current_label: &str,
    expected_label: &str,
) -> String {
    if current == expected {
        return "No content differences.\n".to_string();
    }

    let current_text = String::from_utf8_lossy(current);
    let expected_text = String::from_utf8_lossy(expected);
    let unified = TextDiff::from_lines(current_text.as_ref(), expected_text.as_ref())
        .unified_diff()
        .context_radius(3)
        .header(current_label, expected_label)
        .to_string();
    colorize_unified_diff(&unified)
}

fn colorize_unified_diff(diff: &str) -> String {
    let mut out = String::with_capacity(diff.len());
    for line in diff.split_inclusive('\n') {
        let colored =
            if line.starts_with("@@") || line.starts_with("---") || line.starts_with("+++") {
                colors::cyan(line.trim_end_matches('\n'))
            } else if line.starts_with('+') && !line.starts_with("+++") {
                colors::green(line.trim_end_matches('\n'))
            } else if line.starts_with('-') && !line.starts_with("---") {
                colors::red(line.trim_end_matches('\n'))
            } else {
                line.trim_end_matches('\n').to_string()
            };
        out.push_str(&colored);
        if line.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_file(category: &str, source: &str, dest: &str) -> AppShowFile {
        AppShowFile {
            category: AppCategory {
                name: category.to_string(),
                description: None,
                destination_root: Some("~/.config".to_string()),
                files: vec![],
                list_mode: crate::apps::AppListMode::Files,
                uses_metadata: true,
                has_explicit_files: true,
            },
            file: AppFile {
                source_rel: PathBuf::from(source),
                target_rel: PathBuf::from(source),
                description: None,
                display_name: None,
                legacy_dest_annotation: None,
                transforms: vec![],
                install_strategy: crate::apps::AppInstallStrategy::Copy,
            },
            destination: PathBuf::from(dest),
            status: FileStatus::UpToDate,
            manifest_entry: None,
        }
    }

    fn shell_file(category: &str, command: &str, source: &str) -> ShellShowFile {
        ShellShowFile {
            category: ShellCategory {
                name: category.to_string(),
                description: None,
                files: vec![],
                uses_metadata: true,
            },
            file: crate::shells::metadata::ShellFile {
                source_rel: PathBuf::from(source),
                command_name: command.to_string(),
                description: vec![],
                needs_source: false,
            },
            source_path: PathBuf::from(format!("/tmp/{source}")),
            rendered_path: PathBuf::from(format!("/tmp/rendered/{source}")),
            link_path: PathBuf::from(format!("/tmp/bin/{command}")),
            link_target: None,
            status: "up-to-date",
        }
    }

    #[test]
    fn app_status_sym_matches_list_semantics() {
        assert_eq!(status_sym(FileStatus::UpToDate), "✓");
        assert_eq!(status_sym(FileStatus::UpdateAvail), "↑");
        assert_eq!(status_sym(FileStatus::UserModified), "~");
        assert_eq!(status_sym(FileStatus::Partial), "~");
        assert_eq!(status_sym(FileStatus::Missing), "!");
        assert_eq!(status_sym(FileStatus::NotInstalled), "✗");
    }

    #[test]
    fn shell_status_sym_matches_list_semantics() {
        assert_eq!(shell_status_sym("up-to-date"), "✓");
        assert_eq!(shell_status_sym("update available"), "↑");
        assert_eq!(shell_status_sym("preset present, bin symlink missing"), "~");
        assert_eq!(shell_status_sym("bin symlink present, script missing"), "~");
        assert_eq!(shell_status_sym("bin symlink present, preset missing"), "~");
        assert_eq!(shell_status_sym("rendered script missing"), "!");
        assert_eq!(shell_status_sym("not installed"), "✗");
    }

    #[test]
    fn diff_note_is_returned_for_equal_content() {
        assert_eq!(
            render_diff_or_note(b"same\n", b"same\n", "current", "expected"),
            "No content differences.\n"
        );
    }

    #[test]
    fn diff_output_contains_headers_and_changed_lines() {
        let diff = render_diff_or_note(b"old\n", b"new\n", "current", "expected");
        assert!(diff.contains("current"));
        assert!(diff.contains("expected"));
        assert!(diff.contains("-old"));
        assert!(diff.contains("+new"));
    }

    #[test]
    fn resolves_unique_shell_command_alias() {
        let candidates = build_candidates(&[], &[shell_file("proxy", "setproxy", "set_proxy.sh")]);
        assert_eq!(
            resolve_target("setproxy", &candidates).unwrap(),
            vec![ShowRef::ShellFile {
                category: "proxy".to_string(),
                command: "setproxy".to_string()
            }]
        );
    }

    #[test]
    fn resolves_category_alias() {
        let files = vec![app_file("git", "gitconfig", "/tmp/.gitconfig")];
        let candidates = build_candidates(&files, &[]);
        assert_eq!(
            resolve_target("git", &candidates).unwrap(),
            vec![ShowRef::AppCategory("git".to_string())]
        );
    }

    #[test]
    fn resolves_full_app_file_target() {
        let files = vec![app_file(
            "docker-desktop",
            "settings-store.jsonc",
            "/tmp/settings-store.json",
        )];
        let candidates = build_candidates(&files, &[]);
        assert_eq!(
            resolve_target("app/docker-desktop/settings-store.jsonc", &candidates).unwrap(),
            vec![ShowRef::AppFile {
                category: "docker-desktop".to_string(),
                source: PathBuf::from("settings-store.jsonc")
            }]
        );
    }

    #[test]
    fn resolves_full_shell_command_target() {
        let candidates = build_candidates(&[], &[shell_file("proxy", "setproxy", "set_proxy.sh")]);
        assert_eq!(
            resolve_target("shell/proxy/setproxy", &candidates).unwrap(),
            vec![ShowRef::ShellFile {
                category: "proxy".to_string(),
                command: "setproxy".to_string()
            }]
        );
    }

    #[test]
    fn reports_ambiguous_alias() {
        let app_files = vec![app_file("proxy", "config", "/tmp/proxy")];
        let shell_files = vec![shell_file("proxy", "setproxy", "set_proxy.sh")];
        let candidates = build_candidates(&app_files, &shell_files);
        let err = resolve_target("proxy", &candidates).unwrap_err();
        assert!(err.to_string().contains("ambiguous info target"));
        assert!(err.to_string().contains("app/proxy"));
        assert!(err.to_string().contains("shell/proxy"));
    }

    #[test]
    fn reports_missing_target() {
        let app_files = vec![
            app_file(
                "docker-desktop",
                "settings-store.jsonc",
                "/tmp/settings-store.json",
            ),
            app_file("docker-engine", "daemon.jsonc", "/tmp/daemon.json"),
        ];
        let shell_files = vec![
            shell_file("agent", "ccenv", "cc.sh"),
            shell_file("proxy", "setproxy", "set_proxy.sh"),
            shell_file("tools", "test-tools", "test_tools.sh"),
        ];
        let candidates = build_candidates(&app_files, &shell_files);
        let err = resolve_target("missing", &candidates).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("installed item not found: missing"));
        assert!(message.contains("Available installed targets:"));
        assert!(message.contains("  App Configs"));
        assert!(message.contains("    docker-desktop"));
        assert!(message.contains("    docker-engine"));
        assert!(message.contains("  Shell Presets"));
        assert!(message.contains("    agent"));
        assert!(message.contains("    proxy"));
        assert!(message.contains("    tools"));
        assert!(!message.contains("\n    app/docker-desktop"));
        assert!(!message.contains("\n    shell/proxy"));
        assert!(!message.contains("\n    docker-desktop/settings-store.jsonc"));
        assert!(!message.contains("\n    proxy/setproxy"));
        assert!(message.contains("Run `shine list` to see installed configs."));
        assert!(message.contains(
            "Use full targets like `app/docker-desktop/settings-store.jsonc` for exact file info."
        ));
    }

    #[test]
    fn reports_missing_target_with_suggestions() {
        let app_files = vec![
            app_file(
                "docker-desktop",
                "settings-store.jsonc",
                "/tmp/settings-store.json",
            ),
            app_file("docker-engine", "daemon.jsonc", "/tmp/daemon.json"),
        ];
        let shell_files = vec![shell_file("proxy", "setproxy", "set_proxy.sh")];
        let candidates = build_candidates(&app_files, &shell_files);

        let err = resolve_target("docker", &candidates).unwrap_err();
        let message = err.to_string();

        assert!(message.contains("installed item not found: docker"));
        assert!(message.contains("Did you mean:"));
        assert!(message.contains("  docker-desktop"));
        assert!(message.contains("  docker-engine"));
        assert!(!message.contains("\n  app/docker-desktop"));
        assert!(!message.contains("\n  app/docker-engine"));
        assert!(!message.contains("\n  docker-desktop/settings-store.jsonc"));
        assert!(!message.contains("\n  docker-engine/daemon.jsonc"));
        assert!(!message.contains("\n  proxy"));
        assert!(message.contains("Run `shine list` to see installed configs."));
        assert!(message.contains(
            "Use full targets like `app/docker-desktop/settings-store.jsonc` for exact file info."
        ));
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn renamed_docker_category_has_no_legacy_alias() {
        let dir = std::env::temp_dir().join(format!("shine-show-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let config = Config::new_for_test(&dir);
        tokio::fs::create_dir_all(config.shine_dir()).await.unwrap();

        let files = collect_app_files(&config).await.unwrap();
        assert!(files.iter().all(|file| file.category.name != "docker"));

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }
}
