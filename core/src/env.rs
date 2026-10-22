//! Frontend-neutral environment declaration grammar used by Presets.

use anyhow::{Result, bail};
use std::collections::BTreeSet;

/// Preset input declaration. Persisted launchers and `env run --with` retain
/// their separate string mapping protocol.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(
    deny_unknown_fields,
    expecting = "a structured env entry; run `shine preset migrate --dry-run` for legacy string env"
)]
pub(crate) struct EnvDeclaration {
    pub source: String,
    pub target: Option<String>,
    pub sensitivity: crate::plan::EnvironmentSensitivityV1,
}

pub(crate) fn resolve_env_declarations(
    entries: &[EnvDeclaration],
    permissions: &mut Option<crate::permission::PermissionDeclarationV1>,
) -> Result<Vec<EnvVarSpec>> {
    use crate::permission::{EnvironmentPermissionDeclarationV1, PermissionDeclarationV1};
    let mut targets = BTreeSet::new();
    let mut specs = Vec::new();
    for entry in entries {
        validate_env_key(&entry.source)?;
        let target = entry.target.as_ref().unwrap_or(&entry.source);
        validate_env_key(target)?;
        if !targets.insert(target) {
            bail!("duplicate target variable: {target}");
        }
        let declaration = permissions.get_or_insert_with(|| PermissionDeclarationV1 {
            schema_version: 1,
            opaque_code: None,
            administrator: false,
            filesystem: Vec::new(),
            network: Vec::new(),
            commands: Vec::new(),
            environment: Vec::new(),
            system: Vec::new(),
        });
        if let Some(existing) = declaration
            .environment
            .iter()
            .find(|e| e.name == entry.source)
        {
            if existing.sensitivity != entry.sensitivity {
                bail!("conflicting env sensitivity for {}", entry.source);
            }
        } else {
            declaration
                .environment
                .push(EnvironmentPermissionDeclarationV1 {
                    name: entry.source.clone(),
                    sensitivity: entry.sensitivity,
                });
        }
        specs.push(EnvVarSpec {
            source: entry.source.clone(),
            target: target.clone(),
        });
    }
    Ok(specs)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvVarSpec {
    pub source: String,
    pub target: String,
}

impl EnvVarSpec {
    pub fn to_with_arg(&self) -> String {
        if self.source == self.target {
            self.source.clone()
        } else {
            format!("{}={}", self.source, self.target)
        }
    }
}

pub fn parse_env_specs(specs: &[String]) -> Result<Vec<EnvVarSpec>> {
    let mut parsed = Vec::with_capacity(specs.len());
    let mut targets = BTreeSet::new();
    for spec in specs {
        let (source, target) = spec.split_once('=').unwrap_or((spec, spec));
        validate_env_key(source)?;
        validate_env_key(target)?;
        if !targets.insert(target.to_string()) {
            bail!("duplicate target variable: {target}");
        }
        parsed.push(EnvVarSpec {
            source: source.to_string(),
            target: target.to_string(),
        });
    }
    Ok(parsed)
}

pub fn validate_env_key(key: &str) -> Result<()> {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        bail!("environment variable name must not be empty");
    };
    if !(first == '_' || first.is_ascii_alphabetic())
        || !chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
    {
        bail!("invalid environment variable name: {key}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Deserialize)]
    struct Metadata {
        env: Vec<EnvDeclaration>,
    }

    #[test]
    fn structured_inputs_preserve_alias_order_and_derive_permissions() {
        let metadata: Metadata = toml::from_str("env = [{ source = 'TOKEN', target = 'FIRST', sensitivity = 'secret' }, { source = 'TOKEN', target = 'SECOND', sensitivity = 'secret' }]").unwrap();
        let mut permissions = None;
        let specs = resolve_env_declarations(&metadata.env, &mut permissions).unwrap();
        assert_eq!(
            specs
                .iter()
                .map(EnvVarSpec::to_with_arg)
                .collect::<Vec<_>>(),
            ["TOKEN=FIRST", "TOKEN=SECOND"]
        );
        let permissions = permissions.unwrap();
        assert_eq!(permissions.environment.len(), 1);
        assert_eq!(
            permissions.environment[0].sensitivity,
            crate::plan::EnvironmentSensitivityV1::Secret
        );
        for invalid in [
            "env = [{ source = 'A', target = 'X', sensitivity = 'plain' }, { source = 'B', target = 'X', sensitivity = 'plain' }]",
            "env = [{ source = 'TOKEN', target = 'A', sensitivity = 'plain' }, { source = 'TOKEN', target = 'B', sensitivity = 'secret' }]",
            "env = [{ source = 'BAD-NAME', sensitivity = 'plain' }]",
        ] {
            let metadata: Metadata = toml::from_str(invalid).unwrap();
            assert!(resolve_env_declarations(&metadata.env, &mut None).is_err());
        }
    }

    #[test]
    fn metadata_rejects_legacy_strings_and_incomplete_or_payload_fields() {
        for invalid in [
            "env = ['TOKEN=ALIAS']",
            "env = [{ source = 'TOKEN' }]",
            "env = [{ source = 'TOKEN', sensitivity = 'plain', value = 'not-allowed' }]",
            "env = [{ source = 'TOKEN', sensitivity = 'unknown' }]",
        ] {
            assert!(toml::from_str::<Metadata>(invalid).is_err());
        }
        let error = toml::from_str::<Metadata>("env = ['TOKEN']")
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("shine preset migrate --dry-run"), "{error}");
    }

    #[test]
    fn ordered_specs_reject_duplicate_targets() {
        let error = parse_env_specs(&["A=X".into(), "B=X".into()]).unwrap_err();
        assert!(error.to_string().contains("duplicate target"));
    }
}
