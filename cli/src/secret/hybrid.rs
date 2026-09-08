//! Frozen wire contract: ADR 0084. Parse and bound everything before invoking a tool.
use super::{BackendKind, age, gpg};
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, Payload},
};
use std::{io::IsTerminal, path::PathBuf};
use zeroize::Zeroizing;

pub const PREFIX: &str = "hybrid:";
const KEY_PREFIX: &str = "shine-hybrid-key-v1:";
const MAX_WRAP: usize = 1024 * 1024;
const MAX_DATA: usize = 16 * 1024 * 1024 + 16;
const HEADER: usize = 38;
const MAX_BINARY: usize = HEADER + 2 * MAX_WRAP + MAX_DATA;

#[derive(Clone, Debug)]
pub struct Recipients {
    gpg: Vec<String>,
    age: Vec<String>,
    prepared: bool,
}

impl Recipients {
    pub fn new(gpg: Vec<String>, age: Vec<String>) -> Result<Self> {
        ensure!(
            !gpg.is_empty() && !age.is_empty(),
            "hybrid requires both workspace gpg_recipients and age_recipients"
        );
        for fingerprint in &gpg {
            ensure!(
                fingerprint.len() == 40 && fingerprint.bytes().all(|b| b.is_ascii_hexdigit()),
                "hybrid GPG recipients must be full 40-hex primary fingerprints, not groups or user IDs"
            );
        }
        for recipient in &age {
            ensure!(
                !recipient.is_empty() && !recipient.chars().any(char::is_whitespace),
                "invalid hybrid age recipient"
            );
        }
        Ok(Self {
            gpg,
            age,
            prepared: false,
        })
    }

    /// Runs before any old ciphertext is decrypted. No private-key operation.
    pub async fn prepare(&mut self) -> Result<()> {
        check_version("gpg", &["2.2.", "2.3.", "2.4.", "2.5."]).await?;
        check_version("age", &["1.", "v1."]).await?;
        self.gpg = gpg::resolve_hybrid_recipients(&self.gpg).await?;
        let probe = b"shine hybrid recipient preflight v1";
        gpg::encrypt_hybrid_key(probe, &self.gpg).await?;
        age::encrypt_age_secret_to_base64(probe, &self.age).await?;
        self.prepared = true;
        Ok(())
    }
}

async fn check_version(tool: &str, allowed: &[&str]) -> Result<()> {
    crate::proc::ensure_command(tool)?;
    let output = tokio::process::Command::new(tool)
        .arg("--version")
        .output()
        .await
        .context("checking hybrid encryption tool version")?;
    let text = String::from_utf8_lossy(&output.stdout);
    let version = text
        .lines()
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .last()
        .unwrap_or_default();
    ensure!(
        output.status.success() && allowed.iter().any(|prefix| version.starts_with(prefix)),
        "hybrid sealing requires GnuPG 2.2–2.5 and age 1.x"
    );
    Ok(())
}

struct Envelope {
    bytes: Vec<u8>,
    gpg_end: usize,
    age_end: usize,
}
impl Envelope {
    fn parse(encoded: &str) -> Result<Self> {
        ensure!(
            encoded.len() <= MAX_BINARY.div_ceil(3) * 4,
            "hybrid envelope exceeds size limit"
        );
        let bytes = B64.decode(encoded).context("invalid hybrid Base64")?;
        ensure!(B64.encode(&bytes) == encoded, "noncanonical hybrid Base64");
        ensure!(bytes.len() >= HEADER, "truncated hybrid envelope");
        ensure!(
            bytes[0] == 1 && bytes[1] == 1,
            "unsupported hybrid version or algorithm"
        );
        let length =
            |offset| u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
        let (g, a, d) = (length(26), length(30), length(34));
        ensure!(
            (1..=MAX_WRAP).contains(&g)
                && (1..=MAX_WRAP).contains(&a)
                && (16..=MAX_DATA).contains(&d),
            "invalid hybrid field length"
        );
        ensure!(
            bytes.len() == HEADER + g + a + d,
            "invalid hybrid envelope length or trailing fields"
        );
        Ok(Self {
            bytes,
            gpg_end: HEADER + g,
            age_end: HEADER + g + a,
        })
    }
    fn nonce(&self) -> &[u8] {
        &self.bytes[2..26]
    }
    fn open(&self, key: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
        let cipher = XChaCha20Poly1305::new_from_slice(key)
            .map_err(|_| anyhow::anyhow!("invalid hybrid data key length"))?;
        let plaintext = cipher
            .decrypt(
                XNonce::from_slice(self.nonce()),
                Payload {
                    msg: &self.bytes[self.age_end..],
                    aad: &self.bytes[..self.age_end],
                },
            )
            .map_err(|_| anyhow::anyhow!("hybrid integrity validation failed"))?;
        Ok(Zeroizing::new(plaintext))
    }
}

fn encode(key: &[u8], nonce: &[u8; 24], gpg: &[u8], age: &[u8], plain: &[u8]) -> Result<String> {
    ensure!(
        plain.len() <= MAX_DATA - 16
            && (1..=MAX_WRAP).contains(&gpg.len())
            && (1..=MAX_WRAP).contains(&age.len()),
        "hybrid data or wrapper exceeds size limit"
    );
    let mut bytes = vec![1, 1];
    bytes.extend_from_slice(nonce);
    for len in [gpg.len(), age.len(), plain.len() + 16] {
        bytes.extend_from_slice(&(len as u32).to_be_bytes());
    }
    bytes.extend_from_slice(gpg);
    bytes.extend_from_slice(age);
    let cipher = XChaCha20Poly1305::new_from_slice(key)
        .map_err(|_| anyhow::anyhow!("invalid hybrid data key length"))?;
    let encrypted = cipher
        .encrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: plain,
                aad: &bytes,
            },
        )
        .map_err(|_| anyhow::anyhow!("hybrid encryption failed"))?;
    bytes.extend_from_slice(&encrypted);
    Ok(format!("{PREFIX}{}", B64.encode(bytes)))
}

pub async fn encrypt(plain: &[u8], recipients: &Recipients) -> Result<String> {
    ensure!(
        recipients.prepared,
        "hybrid recipients must pass preflight before decryption or encryption"
    );
    ensure!(
        plain.len() <= MAX_DATA - 16,
        "hybrid plaintext exceeds size limit"
    );
    let mut material = Zeroizing::new([0u8; 56]);
    getrandom::getrandom(&mut *material)
        .map_err(|_| anyhow::anyhow!("OS randomness unavailable"))?;
    let nonce: &[u8; 24] = material[..24].try_into().unwrap();
    let mut wrapped = Zeroizing::new(String::from(KEY_PREFIX));
    // encode_string avoids an unprotected intermediate copy of the key.
    B64.encode_string(material.as_slice(), &mut wrapped);
    let g = gpg::encrypt_hybrid_key(wrapped.as_bytes(), &recipients.gpg).await?;
    let a = age::encrypt_age_secret_to_base64(wrapped.as_bytes(), &recipients.age).await?;
    let a = B64.decode(a).context("invalid age wrapper output")?;
    encode(&material[24..], nonce, &g, &a, plain)
}

async fn select(identities: &[PathBuf], preference: Option<&str>) -> Result<BackendKind> {
    if let Some(preference) = preference {
        return match preference.parse::<BackendKind>()? {
            BackendKind::Hybrid => bail!("hybrid_decrypt_backend must be gpg or age"),
            selected => Ok(selected),
        };
    }
    let gpg = crate::proc::ensure_command("gpg").is_ok();
    let age = age::preflight_identities(identities).await.is_ok();
    match (gpg, age) {
        (true, false) => Ok(BackendKind::Gpg),
        (false, true) => Ok(BackendKind::Age),
        (false, false) => bail!(
            "no hybrid decrypt candidate; install gpg or configure an age identity and its plugins"
        ),
        (true, true) => {
            ensure!(
                std::io::stdin().is_terminal() && std::io::stderr().is_terminal(),
                "both hybrid backends are available; set hybrid_decrypt_backend = \"gpg\" or \"age\" in local global config.toml"
            );
            let choice = dialoguer::Select::new()
                .with_prompt("Decrypt hybrid secret using")
                .items(["GPG", "age"])
                .interact_opt()
                .context("selecting local hybrid backend")?
                .context("hybrid decryption cancelled")?;
            Ok(if choice == 0 {
                BackendKind::Gpg
            } else {
                BackendKind::Age
            })
        }
    }
}

pub async fn decrypt(
    encoded: &str,
    identities: &[PathBuf],
    preference: Option<&str>,
) -> Result<String> {
    let envelope = Envelope::parse(encoded)?;
    let selected = select(identities, preference).await?;
    let wrapped = Zeroizing::new(match selected {
        BackendKind::Gpg => {
            gpg::decrypt_hybrid_key(&B64.encode(&envelope.bytes[HEADER..envelope.gpg_end])).await?
        }
        BackendKind::Age => {
            age::decrypt_hybrid_key(
                &B64.encode(&envelope.bytes[envelope.gpg_end..envelope.age_end]),
                identities,
            )
            .await?
        }
        BackendKind::Hybrid => unreachable!(),
    });
    ensure!(
        wrapped.len() == KEY_PREFIX.len() + 76 && wrapped.starts_with(KEY_PREFIX),
        "invalid hybrid key envelope"
    );
    let material = Zeroizing::new(
        B64.decode(&wrapped[KEY_PREFIX.len()..])
            .context("invalid hybrid key encoding")?,
    );
    ensure!(
        material.len() == 56 && &material[..24] == envelope.nonce(),
        "hybrid key context mismatch"
    );
    let plaintext = envelope.open(&material[24..])?;
    Ok(std::str::from_utf8(&plaintext)
        .context("decrypted hybrid secret is not valid UTF-8")?
        .to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vector_and_all_bytes_are_authenticated() {
        let encoded = encode(
            &[7; 32],
            &[9; 24],
            b"gpg-wrapper",
            b"age-wrapper",
            b"secret\n",
        )
        .unwrap();
        let payload = encoded.strip_prefix(PREFIX).unwrap();
        let envelope = Envelope::parse(payload).unwrap();
        assert_eq!(&**envelope.open(&[7; 32]).unwrap(), b"secret\n");
        // Fixed expected vector is checked in alongside ADR 0084.
        assert_eq!(encoded, include_str!("hybrid-vector.txt").trim());
        for i in 0..envelope.bytes.len() {
            let mut altered = envelope.bytes.clone();
            altered[i] ^= 1;
            if let Ok(parsed) = Envelope::parse(&B64.encode(altered)) {
                assert!(parsed.open(&[7; 32]).is_err(), "byte {i}");
            }
        }
        let mut extra = envelope.bytes.clone();
        extra.push(0);
        assert!(Envelope::parse(&B64.encode(extra)).is_err());
        for size in 0..envelope.bytes.len() {
            assert!(Envelope::parse(&B64.encode(&envelope.bytes[..size])).is_err());
        }
    }
    #[tokio::test]
    async fn malformed_never_selects_tools() {
        for value in ["", "AAAA", "!!!!", "AA==\n"] {
            assert!(
                decrypt(value, &[], Some("invalid-choice"))
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("hybrid")
            );
        }
    }
    #[test]
    fn reject_aliases_and_missing_groups() {
        for value in [
            "team",
            "me@example.org",
            "ABCDEF12",
            "",
            "1234567890123456789012345678901234567890!",
        ] {
            assert!(Recipients::new(vec![value.into()], vec!["age1test".into()]).is_err());
        }
        assert!(Recipients::new(vec![], vec!["age1test".into()]).is_err());
    }
}

#[cfg(all(test, unix))]
mod interoperability {
    use super::*;
    use std::{
        io::Write,
        os::unix::fs::PermissionsExt,
        process::{Command, Stdio},
    };

    struct Sandbox {
        path: PathBuf,
        previous: Option<std::ffi::OsString>,
    }
    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = Command::new("gpgconf")
                .env("GNUPGHOME", &self.path)
                .args(["--kill", "gpg-agent"])
                .output();
            // SAFETY: the test holds the shared environment lock until this guard drops.
            unsafe {
                match &self.previous {
                    Some(value) => std::env::set_var("GNUPGHOME", value),
                    None => std::env::remove_var("GNUPGHOME"),
                }
            }
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
    fn gpg_command(home: &std::path::Path, args: &[&str], input: Option<&[u8]>) -> Vec<u8> {
        let mut child = Command::new("gpg")
            .env("GNUPGHOME", home)
            .args([
                "--no-options",
                "--batch",
                "--pinentry-mode",
                "loopback",
                "--passphrase",
                "",
            ])
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        if let Some(input) = input {
            child.stdin.take().unwrap().write_all(input).unwrap();
        }
        drop(child.stdin.take());
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "GPG test setup failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output.stdout
    }
    fn generate(home: &std::path::Path, name: &str) -> String {
        gpg_command(
            home,
            &["--quick-generate-key", name, "rsa2048", "encr", "0"],
            None,
        );
        let metadata = String::from_utf8(gpg_command(
            home,
            &["--with-colons", "--list-keys", name],
            None,
        ))
        .unwrap();
        metadata
            .lines()
            .find(|line| line.starts_with("fpr:"))
            .unwrap()
            .split(':')
            .nth(9)
            .unwrap()
            .into()
    }

    #[test]
    #[ignore = "requires real GnuPG and age; creates isolated disposable keys"]
    fn real_gpg_age_and_implicit_recipient_isolation() {
        let _lock = crate::test_support::env_lock();
        let path =
            PathBuf::from("/tmp").join(format!("shine-hybrid-real-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let sandbox = Sandbox {
            path: path.clone(),
            previous: std::env::var_os("GNUPGHOME"),
        };
        // SAFETY: serialized by env_lock, with restoration on all exits.
        unsafe {
            std::env::set_var("GNUPGHOME", &path);
        }
        let allowed = generate(&path, "hybrid-allowed");
        let outsider = generate(&path, "hybrid-outsider");
        let foreign = path.join("foreign");
        std::fs::create_dir(&foreign).unwrap();
        std::fs::set_permissions(&foreign, std::fs::Permissions::from_mode(0o700)).unwrap();
        let outsider_key = Zeroizing::new(gpg_command(
            &path,
            &["--export-secret-keys", &outsider],
            None,
        ));
        gpg_command(&foreign, &["--import"], Some(&outsider_key));
        std::fs::write(path.join("gpg.conf"), format!("encrypt-to {outsider}\nhidden-encrypt-to {outsider}\nrecipient {outsider}\ngroup team = {outsider}\ngroup {allowed} = {outsider}\n")).unwrap();
        let identity = path.join("age.txt");
        assert!(
            Command::new("age-keygen")
                .arg("-o")
                .arg(&identity)
                .output()
                .unwrap()
                .status
                .success()
        );
        let public = Command::new("age-keygen")
            .arg("-y")
            .arg(&identity)
            .output()
            .unwrap();
        let public = String::from_utf8(public.stdout).unwrap().trim().to_owned();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let mut recipients = Recipients::new(vec![allowed.clone()], vec![public.clone()]).unwrap();
            recipients.prepare().await.unwrap();
            // Exercise source sealing, both shared broker readers, exports and old-format migration.
            let workspace = path.join("shine.workspace.toml");
            let source = path.join("source.toml");
            std::fs::write(&workspace, format!("version = 2\n[env]\ndefault_mode = 'test'\nfiles = ['source.toml']\n[env.encryption]\nbackend = 'hybrid'\ngpg_recipients = ['{allowed}']\nage_recipients = ['{public}']\n")).unwrap();
            std::fs::write(&source, "[plain]\nPLAIN = 'public'\n[secret]\nTOKEN = 'workspace secret'\n").unwrap();
            let mut config = crate::config::Config::new_for_test(&path);
            config.age_identity = Some(identity.to_string_lossy().into_owned());
            config.hybrid_decrypt_backend = Some("age".into());
            crate::env::workspace::handle_seal(&config, Some(&workspace), None, None, &[]).await.unwrap();
            let snapshot = crate::env::workspace::snapshot_for_broker(Some(&workspace), "test").await.unwrap();
            for backend in ["gpg", "age"] {
                config.hybrid_decrypt_backend = Some(backend.into());
                let values = crate::env::workspace::decrypt_broker_snapshot(&config, &snapshot, &["TOKEN".into()]).await.unwrap();
                assert_eq!(values["TOKEN"], "workspace secret");
            }
            let export = path.join("export.env");
            crate::env::workspace::handle_export(&config, crate::commands::EnvWorkspaceExportFormat::Dotenv,
                Some(&workspace), "test", &export, false, false, false).await.unwrap();
            assert!(!std::fs::read_to_string(&export).unwrap().contains("TOKEN"));
            crate::env::workspace::handle_export(&config, crate::commands::EnvWorkspaceExportFormat::Dotenv,
                Some(&workspace), "test", &export, true, true, false).await.unwrap();
            assert!(std::fs::read_to_string(&export).unwrap().contains("workspace secret"));
            crate::env::workspace::handle_seal(&config, Some(&workspace), None, Some("age"), &[]).await.unwrap();
            assert!(!std::fs::read_to_string(&source).unwrap().contains("hybrid:"));
            crate::env::workspace::handle_seal(&config, Some(&workspace), None, None, &[]).await.unwrap();
            assert!(std::fs::read_to_string(&source).unwrap().contains("hybrid:"));
            let sealed = encrypt(b"same secret\n", &recipients).await.unwrap();
            let encoded = sealed.strip_prefix(PREFIX).unwrap();
            assert_eq!(
                decrypt(
                    encoded,
                    &[PathBuf::from("missing-age-identity")],
                    Some("gpg")
                )
                .await
                .unwrap(),
                "same secret\n"
            );
            assert_eq!(
                decrypt(encoded, std::slice::from_ref(&identity), Some("age"))
                    .await
                    .unwrap(),
                "same secret\n"
            );
            assert!(
                decrypt(encoded, &[PathBuf::from("missing")], Some("age"))
                    .await
                    .is_err()
            );
            // Even a locally configured extra/group recipient cannot unwrap.
            unsafe {
                std::env::set_var("GNUPGHOME", &foreign);
            }
            assert!(
                decrypt(encoded, std::slice::from_ref(&identity), Some("gpg"))
                    .await
                    .is_err()
            );
            unsafe {
                std::env::set_var("GNUPGHOME", &path);
            }
            let second = encrypt(b"other secret", &recipients).await.unwrap();
            let first = Envelope::parse(encoded).unwrap();
            let second = Envelope::parse(second.strip_prefix(PREFIX).unwrap()).unwrap();
            let mut spliced_bytes = first.bytes[..HEADER].to_vec();
            spliced_bytes[26..30]
                .copy_from_slice(&((second.gpg_end - HEADER) as u32).to_be_bytes());
            spliced_bytes.extend_from_slice(&second.bytes[HEADER..second.gpg_end]);
            spliced_bytes.extend_from_slice(&first.bytes[first.gpg_end..]);
            let spliced = B64.encode(spliced_bytes);
            assert!(decrypt(&spliced, &[], Some("gpg")).await.is_err());
        });
        let _ = Command::new("gpgconf")
            .env("GNUPGHOME", &foreign)
            .args(["--kill", "gpg-agent"])
            .output();
        drop(sandbox);
    }
}

#[cfg(all(test, unix))]
mod controlled_process_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    struct Restore(Option<std::ffi::OsString>);
    impl Drop for Restore {
        fn drop(&mut self) {
            // SAFETY: shared environment lock is held until restoration completes.
            unsafe {
                match &self.0 {
                    Some(v) => std::env::set_var("PATH", v),
                    None => std::env::remove_var("PATH"),
                }
            }
        }
    }
    fn tool(dir: &std::path::Path, name: &str, fails: bool) {
        let path = dir.join(name);
        let script = if fails {
            "#!/bin/sh\nexit 42\n"
        } else {
            "#!/bin/sh\nfor arg in \"$@\"; do file=\"$arg\"; done\n/bin/cat \"$file\"\n"
        };
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    #[test]
    fn candidates_cancellation_and_bounded_unwrap() {
        let _lock = crate::test_support::env_lock();
        let _restore = Restore(std::env::var_os("PATH"));
        let dir =
            std::env::temp_dir().join(format!("shine-hybrid-controlled-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let identity = dir.join("identity");
        std::fs::write(&identity, "test").unwrap();
        let mut material = vec![9; 24];
        material.extend_from_slice(&[7; 32]);
        let key = format!("{KEY_PREFIX}{}", B64.encode(material));
        let ciphertext = encode(
            &[7; 32],
            &[9; 24],
            key.as_bytes(),
            key.as_bytes(),
            b"exact\n",
        )
        .unwrap();
        let payload = ciphertext.strip_prefix(PREFIX).unwrap();
        unsafe {
            std::env::set_var("PATH", &dir);
        }
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            assert!(select(&[], None).await.is_err());
            tool(&dir, "gpg", false);
            assert_eq!(
                select(&[PathBuf::from("missing-age")], None).await.unwrap(),
                BackendKind::Gpg
            );
            assert_eq!(decrypt(payload, &[], None).await.unwrap(), "exact\n");
            tool(&dir, "age", false);
            assert_eq!(
                decrypt(payload, std::slice::from_ref(&identity), Some("age"))
                    .await
                    .unwrap(),
                "exact\n"
            );
            if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
                assert!(select(std::slice::from_ref(&identity), None).await.is_err());
            }
            tool(&dir, "gpg", true);
            // age works, but cancellation/failure of selected GPG is terminal.
            assert!(
                decrypt(payload, std::slice::from_ref(&identity), Some("gpg"))
                    .await
                    .is_err()
            );
            std::fs::remove_file(dir.join("gpg")).unwrap();
            assert_eq!(
                select(std::slice::from_ref(&identity), None).await.unwrap(),
                BackendKind::Age
            );
            let oversized =
                encode(&[7; 32], &[9; 24], key.as_bytes(), &[b'x'; 4096], b"secret").unwrap();
            assert!(
                decrypt(
                    oversized.strip_prefix(PREFIX).unwrap(),
                    std::slice::from_ref(&identity),
                    Some("age")
                )
                .await
                .unwrap_err()
                .to_string()
                .contains("limit")
            );
        });
        std::fs::remove_dir_all(dir).unwrap();
    }
}
