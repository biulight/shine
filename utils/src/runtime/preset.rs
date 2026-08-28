use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PresetSourceKind {
    Embedded,
    External,
    Overlay,
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
    files: BTreeMap<String, Vec<u8>>,
}

impl PresetSnapshot {
    pub fn builder(source_kind: PresetSourceKind) -> PresetSnapshotBuilder {
        PresetSnapshotBuilder {
            source_kind,
            files: BTreeMap::new(),
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
    files: BTreeMap<String, Vec<u8>>,
}

impl PresetSnapshotBuilder {
    pub fn file(mut self, path: impl Into<String>, bytes: Vec<u8>) -> Self {
        self.files.insert(path.into(), bytes);
        self
    }

    pub fn build(self) -> PresetSnapshot {
        PresetSnapshot {
            source_kind: self.source_kind,
            files: self.files,
        }
    }
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
