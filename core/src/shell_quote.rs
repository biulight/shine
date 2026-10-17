//! Internal literal encoders shared by CLI rendering and runtime templates.
//! Callers own input validation, path normalization, expression expansion and argv policy.

/// Encode one literal POSIX shell word, including its quote delimiters.
pub fn posix_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Encode a PowerShell single-quoted literal, including its delimiters.
pub fn powershell_single_quote(value: &str) -> String {
    format!("'{}'", powershell_single_quote_contents(value))
}

/// Encode only the contents for templates that already supply quote delimiters.
/// This does not encode a CMD argument or a PowerShell expression.
pub fn powershell_single_quote_contents(value: &str) -> String {
    value.replace('\'', "''")
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn posix_literals_reach_the_shell_as_one_unchanged_argument() {
        for value in [
            "",
            "a b",
            "O'Connor",
            "$HOME/${x}",
            "`cmd`; & | < > ( )",
            "C:\\dir\\file",
            "line\nnext\r\t",
            "中文",
        ] {
            let output = std::process::Command::new("/bin/sh")
                .args([
                    "-c",
                    &format!(
                        "set -- {}; test \"$#\" = 1 && printf '%s' \"$1\"",
                        posix_single_quote(value)
                    ),
                ])
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            assert_eq!(output.stdout, value.as_bytes());
            assert!(output.stderr.is_empty());
        }
    }
}
