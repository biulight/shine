use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::plan::{SnapshotDigestError, SnapshotDigestV1};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PresetSourceKind {
    Embedded,
    External,
    Overlay,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PresetFileOrigin {
    pub source_kind: PresetSourceKind,
    pub physical_path: Option<PathBuf>,
    pub category_root: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PresetFile {
    pub bytes: Vec<u8>,
    pub origin: PresetFileOrigin,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PresetValidationIssue {
    pub code: &'static str,
    pub path: String,
    pub message: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PresetValidationReport {
    pub valid: bool,
    pub categories: Vec<String>,
    pub issues: Vec<PresetValidationIssue>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PresetSnapshot {
    source_kind: PresetSourceKind,
    base: BTreeMap<String, PresetFile>,
    overlay: BTreeMap<String, PresetFile>,
    files: BTreeMap<String, Vec<u8>>,
}

impl PresetSnapshot {
    pub fn builder(source_kind: PresetSourceKind) -> PresetSnapshotBuilder {
        PresetSnapshotBuilder {
            source_kind,
            base_root: None,
            overlay_root: None,
            base: BTreeMap::new(),
            overlay: BTreeMap::new(),
        }
    }

    pub fn source_kind(&self) -> PresetSourceKind {
        self.source_kind
    }

    pub fn files(&self) -> &BTreeMap<String, Vec<u8>> {
        &self.files
    }

    pub fn get(&self, path: &str) -> Option<&[u8]> {
        self.files.get(path).map(Vec::as_slice)
    }

    pub fn file(&self, path: &str) -> Option<&PresetFile> {
        self.overlay.get(path).or_else(|| self.base.get(path))
    }

    pub fn origin(&self, path: &str) -> Option<&PresetFileOrigin> {
        self.file(path).map(|file| &file.origin)
    }

    pub fn is_overlay(&self, path: &str) -> bool {
        self.overlay.contains_key(path)
    }

    /// Bind the effective logical preset tree and trust layer without binding
    /// its machine-local checkout path.
    pub fn digest_v1(&self) -> Result<SnapshotDigestV1, SnapshotDigestError> {
        let mut builder = SnapshotDigestV1::builder("preset");
        builder.add_observation("source-kind", source_kind_name(self.source_kind))?;
        for (path, bytes) in &self.files {
            let origin = self
                .origin(path)
                .map_or(self.source_kind, |origin| origin.source_kind);
            let mut framed = Vec::with_capacity(bytes.len() + 16);
            append_digest_frame(&mut framed, source_kind_name(origin));
            append_digest_frame(&mut framed, bytes);
            builder.add_observation(format!("file:{path}"), framed)?;
        }
        Ok(builder.finish())
    }

    /// Bind selected logical code inputs and their effective trust layers.
    /// Physical checkout locations are intentionally excluded.
    pub fn code_digest_v1<'a>(
        &self,
        paths: impl IntoIterator<Item = &'a str>,
    ) -> Result<SnapshotDigestV1, SnapshotDigestError> {
        let mut builder = SnapshotDigestV1::builder("external-code");
        let paths = paths.into_iter().collect::<BTreeSet<_>>();
        for path in paths {
            let Some(bytes) = self.get(path) else {
                continue;
            };
            let origin = self
                .origin(path)
                .map_or(self.source_kind, |origin| origin.source_kind);
            let mut framed = Vec::with_capacity(bytes.len() + 16);
            append_digest_frame(&mut framed, source_kind_name(origin));
            append_digest_frame(&mut framed, bytes);
            builder.add_observation(format!("file:{path}"), framed)?;
        }
        Ok(builder.finish())
    }

    pub fn validate(&self) -> PresetValidationReport {
        let mut categories = BTreeSet::new();
        let mut issues = Vec::new();
        for path in self.files.keys() {
            let components = path.split('/').collect::<Vec<_>>();
            if components.len() < 3 || !matches!(components[0], "app" | "shell" | "sys") {
                issues.push(PresetValidationIssue {
                    code: "invalid_preset_path",
                    path: path.clone(),
                    message: "expected <app|shell|sys>/<category>/<file>".to_string(),
                });
                continue;
            }
            if components
                .iter()
                .any(|component| component.is_empty() || *component == "." || *component == "..")
            {
                issues.push(PresetValidationIssue {
                    code: "invalid_preset_path",
                    path: path.clone(),
                    message: "preset paths must be normalized and stay inside the snapshot"
                        .to_string(),
                });
                continue;
            }
            categories.insert(format!("{}/{}", components[0], components[1]));
        }
        PresetValidationReport {
            valid: !categories.is_empty() && issues.is_empty(),
            categories: categories.into_iter().collect(),
            issues,
        }
    }
}

pub struct PresetSnapshotBuilder {
    source_kind: PresetSourceKind,
    base_root: Option<PathBuf>,
    overlay_root: Option<PathBuf>,
    base: BTreeMap<String, PresetFile>,
    overlay: BTreeMap<String, PresetFile>,
}

impl PresetSnapshotBuilder {
    pub fn file(mut self, path: impl Into<String>, bytes: Vec<u8>) -> Self {
        let path = normalize_logical_path(path.into());
        let physical_path = self.base_root.as_ref().map(|root| root.join(&path));
        let category_root = self
            .base_root
            .as_deref()
            .and_then(|root| category_root(root, &path));
        self.base.insert(
            path,
            PresetFile {
                bytes,
                origin: PresetFileOrigin {
                    source_kind: self.source_kind,
                    physical_path,
                    category_root,
                },
            },
        );
        self
    }

    pub fn base_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.base_root = Some(root.into());
        self
    }

    pub fn overlay_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.overlay_root = Some(root.into());
        self
    }

    pub fn overlay_file(mut self, path: impl Into<String>, bytes: Vec<u8>) -> Self {
        let path = normalize_logical_path(path.into());
        let physical_path = self.overlay_root.as_ref().map(|root| root.join(&path));
        let category_root = self
            .overlay_root
            .as_deref()
            .and_then(|root| category_root(root, &path));
        self.overlay.insert(
            path,
            PresetFile {
                bytes,
                origin: PresetFileOrigin {
                    source_kind: PresetSourceKind::Overlay,
                    physical_path,
                    category_root,
                },
            },
        );
        self
    }

    pub fn build(self) -> PresetSnapshot {
        let mut files = self
            .base
            .iter()
            .map(|(path, file)| (path.clone(), file.bytes.clone()))
            .collect::<BTreeMap<_, _>>();
        files.extend(
            self.overlay
                .iter()
                .map(|(path, file)| (path.clone(), file.bytes.clone())),
        );
        PresetSnapshot {
            source_kind: self.source_kind,
            base: self.base,
            overlay: self.overlay,
            files,
        }
    }
}

fn normalize_logical_path(path: String) -> String {
    path.replace('\\', "/")
}

fn source_kind_name(source_kind: PresetSourceKind) -> &'static [u8] {
    match source_kind {
        PresetSourceKind::Embedded => b"embedded",
        PresetSourceKind::External => b"external",
        PresetSourceKind::Overlay => b"overlay",
    }
}

fn append_digest_frame(output: &mut Vec<u8>, bytes: &[u8]) {
    output.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
    output.extend_from_slice(bytes);
}

fn category_root(root: &Path, logical_path: &str) -> Option<PathBuf> {
    let mut components = logical_path.split('/');
    let kind = components.next()?;
    let category = components.next()?;
    matches!(kind, "app" | "shell" | "sys").then(|| root.join(kind).join(category))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn immutable_snapshot_rejects_paths_outside_known_kinds() {
        let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
            .file("../secret", Vec::new())
            .build();
        let report = snapshot.validate();
        assert!(!report.valid);
        assert_eq!(report.issues[0].code, "invalid_preset_path");
    }

    #[test]
    fn digest_binds_effective_content_and_layer_but_not_physical_root() {
        let first = PresetSnapshot::builder(PresetSourceKind::External)
            .base_root(PathBuf::from("first-root"))
            .file("app/demo/shine.toml", b"metadata".to_vec())
            .file("app/demo/config.toml", b"content".to_vec())
            .build();
        let relocated = PresetSnapshot::builder(PresetSourceKind::External)
            .base_root(PathBuf::from("other-root"))
            .file("app/demo/shine.toml", b"metadata".to_vec())
            .file("app/demo/config.toml", b"content".to_vec())
            .build();
        let changed_content = PresetSnapshot::builder(PresetSourceKind::External)
            .file("app/demo/shine.toml", b"changed".to_vec())
            .file("app/demo/config.toml", b"content".to_vec())
            .build();
        let overlay = PresetSnapshot::builder(PresetSourceKind::External)
            .file("app/demo/shine.toml", b"metadata".to_vec())
            .file("app/demo/config.toml", b"base".to_vec())
            .overlay_file("app/demo/config.toml", b"content".to_vec())
            .build();

        assert_eq!(first.digest_v1().unwrap(), relocated.digest_v1().unwrap());
        assert_ne!(
            first.digest_v1().unwrap(),
            changed_content.digest_v1().unwrap()
        );
        assert_ne!(first.digest_v1().unwrap(), overlay.digest_v1().unwrap());
    }

    #[test]
    fn code_digest_binds_selected_content_and_effective_layer() {
        let embedded = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file("app/demo/shine.toml", b"metadata".to_vec())
            .file("app/demo/run.sh", b"echo ok".to_vec())
            .build();
        let external = PresetSnapshot::builder(PresetSourceKind::External)
            .file("app/demo/shine.toml", b"metadata".to_vec())
            .file("app/demo/run.sh", b"echo ok".to_vec())
            .build();
        let paths = ["app/demo/shine.toml", "app/demo/run.sh"];
        assert_ne!(
            embedded.code_digest_v1(paths).unwrap(),
            external.code_digest_v1(paths).unwrap()
        );
    }

    #[test]
    fn digest_binds_logical_path_without_host_path_parsing() {
        let first = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file("app/demo/config.toml", b"content".to_vec())
            .build();
        let renamed = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file("app/demo/renamed.toml", b"content".to_vec())
            .build();

        assert_ne!(first.digest_v1().unwrap(), renamed.digest_v1().unwrap());
    }
}
