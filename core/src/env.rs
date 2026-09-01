//! Frontend-neutral environment declaration grammar used by Presets.

use anyhow::{Result, bail};
use std::collections::BTreeSet;

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

    #[test]
    fn ordered_specs_reject_duplicate_targets() {
        let error = parse_env_specs(&["A=X".into(), "B=X".into()]).unwrap_err();
        assert!(error.to_string().contains("duplicate target"));
    }
}
