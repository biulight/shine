//! Observation-only dpkg presence checks shared by planning and execution.
use anyhow::{Context, Result, bail};

pub(super) const STATUS_PATH: &str = "/var/lib/dpkg/status";

pub(super) fn is_present(
    bytes: Option<&[u8]>,
    packages: &[String],
    conflicts: &[String],
) -> Result<bool> {
    let Some(bytes) = bytes else { return Ok(false) };
    let text = std::str::from_utf8(bytes).context("invalid UTF-8 in dpkg status")?;
    let normalized = text.replace("\r\n", "\n");
    let mut installed = std::collections::BTreeSet::new();
    let mut conflicting = false;
    for record in normalized.split("\n\n") {
        let package = record
            .lines()
            .find_map(|line| line.strip_prefix("Package:"))
            .map(str::trim);
        let Some(package) = package else { continue };
        if !packages.iter().chain(conflicts).any(|name| name == package) {
            continue;
        }
        let statuses = record
            .lines()
            .filter_map(|line| line.strip_prefix("Status:"))
            .collect::<Vec<_>>();
        if statuses.len() != 1 {
            bail!("invalid dpkg status for {package}");
        }
        let fields = statuses[0].split_whitespace().collect::<Vec<_>>();
        if fields.len() != 3 {
            bail!("invalid dpkg status for {package}");
        }
        if fields[1] == "ok" && fields[2] == "installed" {
            installed.insert(package);
        }
        if conflicts.iter().any(|name| name == package)
            && !matches!(fields[2], "not-installed" | "config-files")
        {
            conflicting = true;
        }
    }
    Ok(!conflicting
        && packages
            .iter()
            .all(|name| installed.contains(name.as_str())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dpkg_requires_complete_packages_and_no_active_conflicts() {
        let packages = vec!["docker-ce".into(), "docker-compose-plugin".into()];
        let conflicts = vec!["docker.io".into()];
        let complete = "Package: docker-ce\nStatus: hold ok installed\n\nPackage: docker-compose-plugin\nStatus: install ok installed\n";
        assert!(is_present(Some(complete.as_bytes()), &packages, &conflicts).unwrap());
        assert!(
            is_present(
                Some(complete.replace('\n', "\r\n").as_bytes()),
                &packages,
                &conflicts
            )
            .unwrap()
        );
        assert!(!is_present(None, &packages, &conflicts).unwrap());
        assert!(!is_present(Some(b""), &packages, &conflicts).unwrap());
        for state in [
            "unpacked",
            "half-installed",
            "half-configured",
            "triggers-pending",
            "triggers-awaited",
            "installed",
            "config-files",
            "not-installed",
        ] {
            let db = format!("{complete}\nPackage: docker.io\nStatus: deinstall ok {state}\n");
            assert_eq!(
                is_present(Some(db.as_bytes()), &packages, &conflicts).unwrap(),
                matches!(state, "config-files" | "not-installed")
            );
            let partial = complete.replace("hold ok installed", &format!("install ok {state}"));
            assert_eq!(
                is_present(Some(partial.as_bytes()), &packages, &conflicts).unwrap(),
                state == "installed"
            );
        }
        assert!(
            !is_present(
                Some(
                    complete
                        .replace("hold ok installed", "install reinstreq installed")
                        .as_bytes()
                ),
                &packages,
                &conflicts
            )
            .unwrap()
        );
        assert!(
            is_present(
                Some(b"Package: docker-ce\nStatus: broken\n"),
                &packages,
                &conflicts
            )
            .is_err()
        );
        assert!(is_present(Some(b"Package: docker-ce\n"), &packages, &conflicts).is_err());
        assert!(is_present(Some(&[0xff]), &packages, &conflicts).is_err());
    }
}
