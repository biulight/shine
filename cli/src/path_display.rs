use std::path::Path;

pub fn format(path: &Path) -> String {
    normalize(&path.to_string_lossy())
}

pub fn format_home(path: &Path, home_dir: &Path) -> String {
    match path.strip_prefix(home_dir) {
        Ok(relative) if relative.as_os_str().is_empty() => "~".to_string(),
        Ok(relative) => format!("~/{}", normalize(&relative.to_string_lossy())),
        Err(_) => format(path),
    }
}

pub fn format_tilde_path(path: &str, home_dir: &Path) -> String {
    let expanded = crate::config::tilde_expand(path);
    format_home(Path::new(&expanded), home_dir)
}

pub fn strip_windows_verbatim_prefix(value: &str) -> String {
    value
        .strip_prefix(r"\\?\UNC\")
        .map(|path| format!(r"\\{path}"))
        .or_else(|| value.strip_prefix(r"\\?\").map(str::to_string))
        .unwrap_or_else(|| value.to_string())
}

fn normalize(value: &str) -> String {
    strip_windows_verbatim_prefix(value).replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn formats_paths_with_forward_slashes() {
        assert_eq!(
            format(Path::new(r"C:\Users\alice\file.txt")),
            "C:/Users/alice/file.txt"
        );
    }

    #[test]
    fn strips_windows_verbatim_disk_prefix() {
        assert_eq!(
            format(Path::new(r"\\?\C:\Users\alice\file.txt")),
            "C:/Users/alice/file.txt"
        );
    }

    #[test]
    fn strips_windows_verbatim_unc_prefix() {
        assert_eq!(
            format(Path::new(r"\\?\UNC\server\share\file.txt")),
            "//server/share/file.txt"
        );
    }

    #[test]
    fn strips_windows_verbatim_prefix_without_changing_separators() {
        assert_eq!(
            strip_windows_verbatim_prefix(r"\\?\D:\Github\Biulight\shine\preset.ps1"),
            r"D:\Github\Biulight\shine\preset.ps1"
        );
        assert_eq!(
            strip_windows_verbatim_prefix(r"\\?\UNC\server\share\preset.ps1"),
            r"\\server\share\preset.ps1"
        );
    }

    #[test]
    fn collapses_home_prefix() {
        let home = PathBuf::from(r"C:\Users\alice");
        let path = home.join("AppData").join("Roaming").join("Docker");
        assert_eq!(format_home(&path, &home), "~/AppData/Roaming/Docker");
    }

    #[test]
    fn formats_tilde_path_via_home_display_rules() {
        let expanded = crate::config::tilde_expand("~/Library/Application Support");
        let home = PathBuf::from(crate::config::tilde_expand("~"));
        assert_eq!(
            format_tilde_path("~/Library/Application Support", &home),
            format_home(Path::new(&expanded), &home)
        );
    }
}
