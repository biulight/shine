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
        let packages = ["docker-ce".into(), "docker-compose-plugin".into()];
        let conflicts = ["docker.io".into()];
        let present = |bytes: Option<&[u8]>| is_present(bytes, &packages, &conflicts);
        let complete = "Package: docker-ce\nStatus: hold ok installed\n\nPackage: docker-compose-plugin\nStatus: install ok installed\n";
        assert!(present(Some(complete.as_bytes())).unwrap());
        assert!(present(Some(complete.replace('\n', "\r\n").as_bytes())).unwrap());
        assert!(!present(None).unwrap());
        assert!(!present(Some(b"")).unwrap());
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
                present(Some(db.as_bytes())).unwrap(),
                matches!(state, "config-files" | "not-installed"),
                "conflict in state {state}"
            );
            let partial = complete.replace("hold ok installed", &format!("install ok {state}"));
            assert_eq!(
                present(Some(partial.as_bytes())).unwrap(),
                state == "installed",
                "required package in state {state}"
            );
        }
        let broken = complete.replace("hold ok installed", "install reinstreq installed");
        assert!(!present(Some(broken.as_bytes())).unwrap());
        for invalid in [
            b"Package: docker-ce\nStatus: broken\n".as_slice(),
            b"Package: docker-ce\n",
            &[0xff],
        ] {
            assert!(present(Some(invalid)).is_err(), "{invalid:?}");
        }
    }
}
