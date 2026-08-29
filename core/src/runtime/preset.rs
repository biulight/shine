use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

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
}
