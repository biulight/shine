//! Versioned authoring declarations for Preset permissions.
//!
//! These declarations describe reviewable capability identities. They do not
//! grant execution permission and are deliberately separate from the
//! snapshot-bound [`crate::plan::PlanV1`] wire contract.

use crate::plan::{
    EnvironmentSensitivityV1, FilesystemAccessV1, NetworkScopeV1, PermissionSetV1, PermissionV1,
};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::fmt;

pub const PERMISSION_DECLARATION_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PermissionDeclarationV1 {
    pub schema_version: u32,
    #[serde(default)]
    pub administrator: bool,
    #[serde(default)]
    pub filesystem: Vec<FilesystemPermissionDeclarationV1>,
    #[serde(default)]
    pub network: Vec<NetworkPermissionDeclarationV1>,
    #[serde(default)]
    pub commands: Vec<String>,
    #[serde(default)]
    pub environment: Vec<EnvironmentPermissionDeclarationV1>,
    #[serde(default)]
    pub system: Vec<SystemPermissionDeclarationV1>,
}

impl PermissionDeclarationV1 {
    pub fn validate(&self) -> Result<(), PermissionDeclarationError> {
        if self.schema_version != PERMISSION_DECLARATION_SCHEMA_VERSION {
            return Err(PermissionDeclarationError::UnsupportedSchema(
                self.schema_version,
            ));
        }

        let mut filesystem = BTreeSet::new();
        for declaration in &self.filesystem {
            declaration.validate()?;
            let path = normalize_declared_path(&declaration.path);
            let mut access = BTreeSet::new();
            for item in &declaration.access {
                if !access.insert(*item) {
                    return Err(PermissionDeclarationError::Duplicate(format!(
                        "filesystem access `{}` for {}:{}",
                        filesystem_access_name(*item),
                        declaration.base.as_str(),
                        declaration.path
                    )));
                }
                if !filesystem.insert((declaration.base, path.clone(), *item)) {
                    return Err(PermissionDeclarationError::Duplicate(format!(
                        "filesystem permission `{}` for {}:{}",
                        filesystem_access_name(*item),
                        declaration.base.as_str(),
                        declaration.path
                    )));
                }
            }
        }

        let mut network = BTreeSet::new();
        for declaration in &self.network {
            declaration.validate()?;
            let key = (
                declaration.scope,
                declaration.host.as_deref().map(normalize_host),
            );
            if !network.insert(key) {
                return Err(PermissionDeclarationError::Duplicate(
                    "network permission".to_string(),
                ));
            }
        }

        let mut commands = BTreeSet::new();
        for command in &self.commands {
            validate_program(command)?;
            if !commands.insert(command) {
                return Err(PermissionDeclarationError::Duplicate(format!(
                    "command `{command}`"
                )));
            }
        }

        let mut environment = BTreeSet::new();
        for declaration in &self.environment {
            validate_env_name(&declaration.name)?;
            if !environment.insert(&declaration.name) {
                return Err(PermissionDeclarationError::Duplicate(format!(
                    "environment variable `{}`",
                    declaration.name
                )));
            }
        }

        let mut system = BTreeSet::new();
        for declaration in &self.system {
            declaration.validate()?;
            if !system.insert((declaration.capability.clone(), declaration.resource.clone())) {
                return Err(PermissionDeclarationError::Duplicate(format!(
                    "system capability `{}`",
                    declaration.capability
                )));
            }
        }
        Ok(())
    }

    /// Normalize one author declaration into the same sorted, duplicate-free
    /// permission vocabulary used by a security Plan. Filesystem paths retain
    /// their logical base and never contain a physical Preset checkout root.
    pub fn permission_set(&self) -> Result<PermissionSetV1, PermissionDeclarationError> {
        self.validate()?;
        let mut permissions = Vec::new();
        if self.administrator {
            permissions.push(PermissionV1::Administrator);
        }
        for declaration in &self.filesystem {
            let path = format!(
                "{}:{}",
                declaration.base.as_str(),
                normalize_declared_path(&declaration.path)
            );
            permissions.extend(
                declaration
                    .access
                    .iter()
                    .map(|access| PermissionV1::Filesystem {
                        access: *access,
                        path: path.clone(),
                    }),
            );
        }
        for declaration in &self.network {
            let scope = match declaration.scope {
                DeclaredNetworkScopeV1::Any => NetworkScopeV1::Any,
                DeclaredNetworkScopeV1::Host => NetworkScopeV1::Host(normalize_host(
                    declaration.host.as_deref().expect("validated network host"),
                )),
            };
            permissions.push(PermissionV1::Network { scope });
        }
        permissions.extend(
            self.commands
                .iter()
                .cloned()
                .map(|program| PermissionV1::Command { program }),
        );
        permissions.extend(
            self.environment
                .iter()
                .map(|declaration| PermissionV1::Environment {
                    name: declaration.name.clone(),
                    sensitivity: declaration.sensitivity,
                }),
        );
        permissions.extend(self.system.iter().map(|declaration| PermissionV1::System {
            capability: declaration.capability.clone(),
            resource: declaration.resource.clone(),
        }));
        Ok(PermissionSetV1::new(permissions))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FilesystemPermissionDeclarationV1 {
    pub access: Vec<FilesystemAccessV1>,
    pub base: PermissionPathBaseV1,
    pub path: String,
}

impl FilesystemPermissionDeclarationV1 {
    fn validate(&self) -> Result<(), PermissionDeclarationError> {
        if self.access.is_empty() {
            return Err(PermissionDeclarationError::Invalid(
                "filesystem access must not be empty".to_string(),
            ));
        }
        match self.base {
            PermissionPathBaseV1::Absolute => validate_absolute_path(&self.path),
            _ => validate_relative_path(&self.path),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(rename_all = "kebab-case")]
pub enum PermissionPathBaseV1 {
    Home,
    Shine,
    DataDir,
    Preset,
    Absolute,
}

impl PermissionPathBaseV1 {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Home => "home",
            Self::Shine => "shine",
            Self::DataDir => "data-dir",
            Self::Preset => "preset",
            Self::Absolute => "absolute",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NetworkPermissionDeclarationV1 {
    pub scope: DeclaredNetworkScopeV1,
    pub host: Option<String>,
}

impl NetworkPermissionDeclarationV1 {
    fn validate(&self) -> Result<(), PermissionDeclarationError> {
        match (self.scope, self.host.as_deref()) {
            (DeclaredNetworkScopeV1::Any, None) => Ok(()),
            (DeclaredNetworkScopeV1::Any, Some(_)) => Err(PermissionDeclarationError::Invalid(
                "network scope `any` must not declare a host".to_string(),
            )),
            (DeclaredNetworkScopeV1::Host, Some(host)) => validate_host(host),
            (DeclaredNetworkScopeV1::Host, None) => Err(PermissionDeclarationError::Invalid(
                "network scope `host` requires a host".to_string(),
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(rename_all = "kebab-case")]
pub enum DeclaredNetworkScopeV1 {
    Any,
    Host,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentPermissionDeclarationV1 {
    pub name: String,
    pub sensitivity: EnvironmentSensitivityV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SystemPermissionDeclarationV1 {
    pub capability: String,
    pub resource: Option<String>,
}

impl SystemPermissionDeclarationV1 {
    fn validate(&self) -> Result<(), PermissionDeclarationError> {
        if !is_capability_identifier(&self.capability) {
            return Err(PermissionDeclarationError::Invalid(format!(
                "system capability `{}` must use lowercase letters, digits, `.`, `_`, or `-`",
                self.capability
            )));
        }
        if self
            .resource
            .as_deref()
            .is_some_and(|resource| resource.is_empty() || contains_control(resource))
        {
            return Err(PermissionDeclarationError::Invalid(
                "system resource must be a non-empty single-line identity".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PermissionDeclarationError {
    UnsupportedSchema(u32),
    Invalid(String),
    Duplicate(String),
}

impl PermissionDeclarationError {
    pub const fn diagnostic_code(&self) -> &'static str {
        match self {
            Self::UnsupportedSchema(_) => "unsupported_permission_schema",
            Self::Invalid(_) => "invalid_permission_declaration",
            Self::Duplicate(_) => "duplicate_permission",
        }
    }
}

impl fmt::Display for PermissionDeclarationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSchema(version) => {
                write!(formatter, "unsupported permission schema version {version}")
            }
            Self::Invalid(message) => {
                write!(formatter, "invalid permission declaration: {message}")
            }
            Self::Duplicate(permission) => {
                write!(formatter, "duplicate permission declaration: {permission}")
            }
        }
    }
}

impl std::error::Error for PermissionDeclarationError {}

fn validate_relative_path(path: &str) -> Result<(), PermissionDeclarationError> {
    if path == "." {
        return Ok(());
    }
    if path.is_empty()
        || path.starts_with(['/', '\\'])
        || is_windows_drive_path(path)
        || path.contains('\\')
        || path
            .split('/')
            .any(|component| component.is_empty() || matches!(component, "." | ".."))
        || contains_control(path)
    {
        return Err(PermissionDeclarationError::Invalid(format!(
            "permission path `{path}` must be a normalized relative path"
        )));
    }
    Ok(())
}

fn validate_absolute_path(path: &str) -> Result<(), PermissionDeclarationError> {
    if path.is_empty()
        || !is_portable_absolute_path(path)
        || path.split(['/', '\\']).any(|component| component == "..")
        || contains_control(path)
    {
        return Err(PermissionDeclarationError::Invalid(format!(
            "permission path `{path}` must be a portable absolute path without `..`"
        )));
    }
    Ok(())
}

fn validate_host(host: &str) -> Result<(), PermissionDeclarationError> {
    if host.is_empty()
        || host.contains("://")
        || host.contains(['/', '@', '?', '#'])
        || host.chars().any(char::is_whitespace)
        || contains_control(host)
    {
        return Err(PermissionDeclarationError::Invalid(format!(
            "network host `{host}` must not contain a URL scheme, path, credentials, or whitespace"
        )));
    }
    Ok(())
}

fn validate_program(program: &str) -> Result<(), PermissionDeclarationError> {
    if program.is_empty() || program.chars().any(char::is_whitespace) || contains_control(program) {
        return Err(PermissionDeclarationError::Invalid(format!(
            "command `{program}` must be one program identity without arguments"
        )));
    }
    Ok(())
}

fn validate_env_name(name: &str) -> Result<(), PermissionDeclarationError> {
    let mut characters = name.chars();
    let valid_start = characters
        .next()
        .is_some_and(|character| character == '_' || character.is_ascii_alphabetic());
    if !valid_start
        || !characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
    {
        return Err(PermissionDeclarationError::Invalid(format!(
            "environment variable `{name}` is not a portable variable name"
        )));
    }
    Ok(())
}

fn is_capability_identifier(value: &str) -> bool {
    value
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
        && value.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '.' | '_' | '-')
        })
}

fn is_portable_absolute_path(path: &str) -> bool {
    path.starts_with('/')
        || path.starts_with("\\\\")
        || (path.len() >= 3
            && path.as_bytes()[0].is_ascii_alphabetic()
            && path.as_bytes()[1] == b':'
            && matches!(path.as_bytes()[2], b'/' | b'\\'))
}

fn is_windows_drive_path(path: &str) -> bool {
    path.len() >= 2 && path.as_bytes()[0].is_ascii_alphabetic() && path.as_bytes()[1] == b':'
}

fn contains_control(value: &str) -> bool {
    value.chars().any(char::is_control)
}

fn normalize_declared_path(path: &str) -> String {
    if path == "." {
        return path.to_string();
    }
    let path = path.replace('\\', "/");
    let (prefix, remainder) = if let Some(remainder) = path.strip_prefix("//") {
        ("//", remainder)
    } else if let Some(remainder) = path.strip_prefix('/') {
        ("/", remainder)
    } else if path.len() >= 3 && path.as_bytes()[1] == b':' && path.as_bytes()[2] == b'/' {
        (&path[..3], &path[3..])
    } else {
        ("", path.as_str())
    };
    let remainder = remainder
        .split('/')
        .filter(|component| !component.is_empty() && *component != ".")
        .collect::<Vec<_>>()
        .join("/");
    format!("{prefix}{remainder}")
}

fn normalize_host(host: &str) -> String {
    host.to_ascii_lowercase()
}

fn filesystem_access_name(access: FilesystemAccessV1) -> &'static str {
    match access {
        FilesystemAccessV1::Read => "read",
        FilesystemAccessV1::Write => "write",
        FilesystemAccessV1::Remove => "remove",
        FilesystemAccessV1::Execute => "execute",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(value: &str) -> Result<PermissionDeclarationV1, toml::de::Error> {
        toml::from_str(value)
    }

    #[test]
    fn complete_declaration_normalizes_to_plan_permissions() {
        let declaration = parse(
            r#"
schema_version = 1
administrator = true
filesystem = [
  { access = ["write", "read"], base = "home", path = ".config/example" },
  { access = ["execute"], base = "preset", path = "build.ts" },
]
network = [
  { scope = "host", host = "api.example.com" },
  { scope = "any" },
]
commands = ["bun"]
environment = [{ name = "API_TOKEN", sensitivity = "secret" }]
system = [{ capability = "split-dns", resource = "private-domain" }]
"#,
        )
        .unwrap();

        declaration.validate().unwrap();
        let permissions = declaration.permission_set().unwrap();
        assert_eq!(permissions.iter().count(), 9);
        assert!(permissions.contains(&PermissionV1::Filesystem {
            access: FilesystemAccessV1::Read,
            path: "home:.config/example".to_string(),
        }));
        assert!(permissions.contains(&PermissionV1::Environment {
            name: "API_TOKEN".to_string(),
            sensitivity: EnvironmentSensitivityV1::Secret,
        }));
    }

    #[test]
    fn schema_and_unknown_fields_fail_closed() {
        let declaration = parse("schema_version = 2\n").unwrap();
        assert_eq!(
            declaration.validate().unwrap_err().diagnostic_code(),
            "unsupported_permission_schema"
        );
        assert!(parse("schema_version = 1\nsecret_value = 'nope'\n").is_err());
    }

    #[test]
    fn duplicate_permissions_are_rejected() {
        let declaration = parse(
            r#"
schema_version = 1
commands = ["bun", "bun"]
"#,
        )
        .unwrap();
        assert_eq!(
            declaration.validate().unwrap_err().diagnostic_code(),
            "duplicate_permission"
        );

        for value in [
            r#"
schema_version = 1
filesystem = [
  { access = ["read"], base = "absolute", path = "C:/Data/config" },
  { access = ["read"], base = "absolute", path = "C:\\Data\\config" },
]
"#,
            r#"
schema_version = 1
network = [
  { scope = "host", host = "API.EXAMPLE.COM" },
  { scope = "host", host = "api.example.com" },
]
"#,
        ] {
            assert_eq!(
                parse(value)
                    .unwrap()
                    .validate()
                    .unwrap_err()
                    .diagnostic_code(),
                "duplicate_permission"
            );
        }
    }

    #[test]
    fn path_bases_enforce_relative_and_absolute_contracts() {
        for value in [
            r#"schema_version = 1
filesystem = [{ access = ["read"], base = "home", path = "../secret" }]
"#,
            r#"schema_version = 1
filesystem = [{ access = ["write"], base = "absolute", path = "relative/file" }]
"#,
        ] {
            assert_eq!(
                parse(value)
                    .unwrap()
                    .validate()
                    .unwrap_err()
                    .diagnostic_code(),
                "invalid_permission_declaration"
            );
        }
        for path in [
            "/etc/example",
            r"C:\\ProgramData\\example",
            r"\\\\server\\share",
        ] {
            validate_absolute_path(path).unwrap();
        }
    }

    #[test]
    fn host_program_environment_and_system_identities_are_payload_free() {
        for value in [
            r#"schema_version = 1
network = [{ scope = "host", host = "https://api.example.com/path" }]
"#,
            r#"schema_version = 1
commands = ["sh -c"]
"#,
            r#"schema_version = 1
environment = [{ name = "TOKEN=value", sensitivity = "secret" }]
"#,
            r#"schema_version = 1
system = [{ capability = "Split DNS" }]
"#,
        ] {
            assert_eq!(
                parse(value)
                    .unwrap()
                    .validate()
                    .unwrap_err()
                    .diagnostic_code(),
                "invalid_permission_declaration"
            );
        }
    }
}
