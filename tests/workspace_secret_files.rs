//! CLI safety regressions. The fake backend is a byte transport, not cryptography.
use std::{
    fs,
    path::PathBuf,
    process::{Child, Command, Output, Stdio},
    sync::LazyLock,
    time::{Duration, Instant},
};

#[cfg(unix)]
use std::path::Path;

const PROBE: &str = r#"
use std::{env,fs,io::{Read,Write},process::Command,time::Duration};
fn main() {
 let args:Vec<_>=env::args_os().skip(1).collect();
 if env::current_exe().unwrap().file_stem().unwrap()=="age" {
  if args.iter().any(|a| a=="--version") { println!("1.3.0"); return; }
  eprintln!("phone-authorization-prompt");
  if env::var_os("PROBE_FAIL_DECRYPT").is_some() { std::process::exit(2); }
  if fs::read(args.last().unwrap()).unwrap().ends_with(b"empty") { return; }
  std::io::stdout().write_all(&[0,255,10,13,128]).unwrap();
  return;
 }
 if env::current_exe().unwrap().file_stem().unwrap()=="gpg" {
  if let Some(log)=env::var_os("PROBE_BACKEND_LOG") { fs::write(log,"called").unwrap(); }
  if let Some(path)=env::var_os("PROBE_MUTATE") { fs::OpenOptions::new().append(true).open(path).unwrap().write_all(b"\n# changed\n").unwrap(); }
  if args.iter().any(|a| a=="--decrypt") {
   if let Some(ready)=env::var_os("PROBE_READY") {
    fs::write(ready, "ready").unwrap();
    let release=env::var_os("PROBE_RELEASE").unwrap();
    let deadline=std::time::Instant::now()+Duration::from_secs(20);
    while !std::path::Path::new(&release).exists() { assert!(std::time::Instant::now()<deadline); std::thread::sleep(Duration::from_millis(10)); }
   }
   if env::var_os("PROBE_FAIL_DECRYPT").is_some() { eprintln!("must-not-leak-plaintext"); std::process::exit(2); }
   if let Some(path)=env::var_os("PROBE_FAIL_SECOND") {
    if std::path::Path::new(&path).exists() { std::process::exit(2); }
    fs::write(path,"first").unwrap();
   }
   let bytes=fs::read(args.last().unwrap()).unwrap();
   std::io::stdout().write_all(&bytes[19..]).unwrap();
  } else {
   let mut input=Vec::new();std::io::stdin().read_to_end(&mut input).unwrap();
   let mut output=vec![0xc1,10,3,0,0,0,0,0,0,0,0,1,0xd2,0xff];
   output.extend_from_slice(&((input.len()+1) as u32).to_be_bytes()); output.push(1);output.extend(input);
   std::io::stdout().write_all(&output).unwrap();
  }
  return;
 }
 match args[0].to_str().unwrap() {
  "check" => { assert_eq!(fs::read(".signing/key").unwrap(), [0,255,10,13,128]); assert!(fs::read("other/password").unwrap().is_empty()); }
  "output" => { std::io::stdout().write_all(&[0,255,10,13,128]).unwrap(); }
  "remove" => { fs::remove_file(".signing/key").unwrap(); }
  "remove-parent" => { fs::remove_dir_all(".signing").unwrap(); }
  "replace-directory" => { fs::remove_file(".signing/key").unwrap(); fs::create_dir(".signing/key").unwrap(); }
  #[cfg(unix)]
  "replace-dangling-link" => { fs::remove_file(".signing/key").unwrap(); std::os::unix::fs::symlink("missing", ".signing/key").unwrap(); }
  "replace" => { fs::remove_file(".signing/key").unwrap(); fs::write(".signing/key","replacement").unwrap(); }
  #[cfg(unix)]
  "unreadable" => { use std::os::unix::fs::PermissionsExt; fs::set_permissions(".signing/key", fs::Permissions::from_mode(0)).unwrap(); }
  #[cfg(unix)]
  "replace-unreadable" => { use std::os::unix::fs::PermissionsExt; fs::remove_file(".signing/key").unwrap(); fs::write(".signing/key","replacement").unwrap(); fs::set_permissions(".signing/key", fs::Permissions::from_mode(0)).unwrap(); }
  "wait" => {
   let child=Command::new(env::current_exe().unwrap()).arg("descendant").stdin(std::process::Stdio::null()).spawn().unwrap();
   fs::write("ready",child.id().to_string()).unwrap();
   loop { std::thread::sleep(Duration::from_millis(20)); }
  }
  "descendant" => { loop { std::thread::sleep(Duration::from_millis(20)); } }
  _ => panic!("unexpected probe mode"),
 }
 if let Some(code)=env::var_os("PROBE_EXIT") { std::process::exit(code.to_str().unwrap().parse().unwrap()); }
}
"#;

struct Probe(PathBuf);
impl Drop for Probe {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
static PROBE_BINARY: LazyLock<Probe> = LazyLock::new(|| {
    let dir = std::env::temp_dir().join(format!("shine-file-probe-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("probe.rs"), PROBE).unwrap();
    let output = Command::new("rustc")
        .arg("--edition=2024")
        .arg(dir.join("probe.rs"))
        .arg("-o")
        .arg(dir.join(exe("probe")))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    fs::copy(dir.join(exe("probe")), dir.join(exe("gpg"))).unwrap();
    fs::copy(dir.join(exe("probe")), dir.join(exe("age"))).unwrap();
    fs::copy(dir.join(exe("probe")), dir.join(exe("age-plugin-phone"))).unwrap();
    Probe(dir)
});
fn exe(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}
struct Fixture {
    root: PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("shine-file-groups-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("state")).unwrap();
        fs::write(root.join("state/config.toml"), "schema_version=2\n").unwrap();
        fs::write(root.join("shine.workspace.toml"), "version=3\n[env]\ndefault_mode='test'\nmodes=['test','prod']\n[env.encryption]\nbackend='gpg'\ngpg_recipients=['test']\n[[file_groups.signing.files]]\nsource='.secrets/{mode}/key.enc'\ntarget='.signing/key'\n[[file_groups.account.files]]\nsource='.secrets/{mode}/password.enc'\ntarget='other/password'\n").unwrap();
        fs::create_dir(root.join(".signing")).unwrap();
        fs::create_dir(root.join("other")).unwrap();
        fs::write(root.join(".signing/key"), [0, 255, 10, 13, 128]).unwrap();
        fs::write(root.join("other/password"), []).unwrap();
        Self { root }
    }
    fn command(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_shine"));
        cmd.current_dir(&self.root)
            .env("SHINE_CONFIG_DIR", self.root.join("state"))
            .env(
                "PATH",
                std::env::join_paths(std::iter::once(PROBE_BINARY.0.clone()).chain(
                    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
                ))
                .unwrap(),
            );
        cmd
    }
    fn seal(&self, extra: &[&str]) -> Output {
        self.command()
            .args([
                "env",
                "secret",
                "seal",
                "--file-group",
                "signing",
                "--file-group",
                "account",
            ])
            .args(extra)
            .output()
            .unwrap()
    }
    fn run(&self, mode: &str) -> Command {
        let mut cmd = self.command();
        cmd.args([
            "env",
            "run",
            "--file-group",
            "signing",
            "--file-group",
            "account",
            "--",
        ])
        .arg(PROBE_BINARY.0.join(exe("probe")))
        .arg(mode);
        cmd
    }
    fn export(&self) -> Command {
        let mut cmd = self.command();
        cmd.args([
            "env",
            "workspace",
            "export",
            "--format",
            "files",
            "--mode",
            "test",
            "--file-group",
            "signing",
            "--file-group",
            "account",
        ]);
        cmd
    }
    fn blocked_export(&self) -> Child {
        self.export()
            .env("PROBE_READY", self.root.join("export-ready"))
            .env("PROBE_RELEASE", self.root.join("export-release"))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    }
    fn await_export(&self, child: &mut Child) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while !self.root.join("export-ready").exists() {
            assert!(
                child.try_wait().unwrap().is_none(),
                "export exited before backend barrier"
            );
            assert!(
                Instant::now() < deadline,
                "export never reached backend barrier"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn release_export(&self) {
        fs::write(self.root.join("export-release"), "go").unwrap();
    }
    fn prepare(&self) {
        good(self.seal(&["--remove-plaintext"]));
        fs::remove_dir(self.root.join(".signing")).unwrap();
        fs::remove_dir(self.root.join("other")).unwrap();
    }
}
fn good(output: Output) {
    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
fn error(output: Output, expected: &str) {
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(expected),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(unix)]
#[test]
fn file_groups_clean_unreadable_owned_plaintext_before_propagating_child_status() {
    for code in [0, 42] {
        let f = Fixture::new();
        f.prepare();
        let output = f
            .run("unreadable")
            .env("PROBE_EXIT", code.to_string())
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(code),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!f.root.join(".signing/key").exists());
        assert!(!f.root.join(".signing").exists());
        assert!(!f.root.join("other").exists());
    }
    // Permission-independent inspection must still preserve a substituted file.
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    f.prepare();
    error(
        f.run("replace-unreadable").output().unwrap(),
        "plaintext target was replaced",
    );
    let path = f.root.join(".signing/key");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(fs::read(path).unwrap(), b"replacement");
}

#[test]
fn file_groups_age_keeps_phone_authorization_interactive_and_stdout_binary() {
    use base64::Engine;
    let f = Fixture::new();
    f.prepare();
    let identity = f.root.join("identity.txt");
    fs::write(&identity, "AGE-PLUGIN-PHONE-1EXAMPLE\n").unwrap();
    let mut config: toml::Table = toml::from_str("schema_version=2").unwrap();
    config.insert(
        "age_identity".into(),
        identity.to_string_lossy().as_ref().into(),
    );
    fs::write(
        f.root.join("state/config.toml"),
        toml::to_string(&config).unwrap(),
    )
    .unwrap();
    let cipher = format!(
        "age:{}",
        base64::engine::general_purpose::STANDARD.encode(b"age-encryption.org/v1\nfixture")
    );
    for name in ["key", "password"] {
        fs::write(f.root.join(format!(".secrets/test/{name}.enc")), &cipher).unwrap();
    }
    for (transport, messages, visible) in [
        ("qr", "0", true),
        ("adb", "0", false),
        ("adb", "true", true),
        ("auto", "0", !cfg!(windows)),
    ] {
        let output = f
            .run("output")
            .env("AGE_PLUGIN_PHONE_TRANSPORT", transport)
            .env("AGE_PLUGIN_PHONE_MESSAGES", messages)
            .output()
            .unwrap();
        assert_eq!(output.stdout, [0, 255, 10, 13, 128]);
        assert_eq!(
            String::from_utf8_lossy(&output.stderr).contains("phone-authorization-prompt"),
            visible,
            "transport={transport}, messages={messages}"
        );
        good(output);
        assert!(!f.root.join(".signing").exists());
    }
    // Ordinary backend diagnostics remain suppressed, including on failure.
    fs::write(&identity, "AGE-SECRET-KEY-EXAMPLE\n").unwrap();
    let output = f
        .run("output")
        .env("AGE_PLUGIN_PHONE_TRANSPORT", "qr")
        .env("PROBE_FAIL_DECRYPT", "1")
        .output()
        .unwrap();
    assert!(!String::from_utf8_lossy(&output.stderr).contains("phone-authorization-prompt"));
    assert!(output.stdout.is_empty());
    error(output, "file decryption failed");
    assert!(!f.root.join(".signing").exists());
}

#[test]
fn file_groups_seal_and_run_binary_empty_multiple_modes_and_exit_codes() {
    let f = Fixture::new();
    good(f.seal(&[]));
    assert!(f.root.join(".signing/key").exists());
    // Updating existing ciphertext must also work while snapshots protect the inputs.
    good(f.seal(&[]));
    good(f.seal(&["--mode", "prod"]));
    assert!(f.root.join(".secrets/prod/key.enc").exists());
    f.prepare();
    good(f.run("check").output().unwrap());
    assert!(!f.root.join(".signing").exists());
    assert!(!f.root.join("other").exists());
    let out = f.run("check").env("PROBE_EXIT", "23").output().unwrap();
    assert_eq!(out.status.code(), Some(23));
    assert!(!f.root.join(".signing/key").exists());
    let out = f
        .command()
        .args([
            "env",
            "run",
            "--file-group",
            "signing",
            "--",
            "shine-no-such-file-command",
        ])
        .output()
        .unwrap();
    error(out, "starting file-group command");
    assert!(!f.root.join(".signing/key").exists());
}

#[test]
fn file_groups_preview_never_calls_backend_or_command() {
    let f = Fixture::new();
    let log = f.root.join("backend-log");
    good(
        f.command()
            .env("PROBE_BACKEND_LOG", &log)
            .args([
                "env",
                "secret",
                "seal",
                "--file-group",
                "signing",
                "--dry-run",
            ])
            .output()
            .unwrap(),
    );
    assert!(!log.exists());
    assert!(!f.root.join(".secrets").exists());
    let untouched_state = f.root.join("uninitialized-state");
    good(
        f.command()
            .env("SHINE_CONFIG_DIR", &untouched_state)
            .args([
                "env",
                "secret",
                "seal",
                "--file-group",
                "signing",
                "--dry-run",
            ])
            .output()
            .unwrap(),
    );
    assert!(!untouched_state.exists());
    f.prepare();
    good(
        f.command()
            .env("PROBE_BACKEND_LOG", &log)
            .args([
                "env",
                "run",
                "--file-group",
                "signing",
                "--dry-run",
                "--",
                "nonexistent-command",
            ])
            .output()
            .unwrap(),
    );
    assert!(!log.exists());
    assert!(!f.root.join(".signing").exists());
}

#[test]
fn file_group_previews_escape_paths_without_changing_io_or_streams() {
    for target in [
        ".signing/key with space.bin",
        ".signing/it's.bin",
        #[cfg(unix)]
        ".signing/key\nnext\t.bin",
    ] {
        let mut f = Fixture::new();
        let root = f.root.with_extension(if cfg!(unix) {
            "display\nFORGED=path\r\t\u{1b}中文"
        } else {
            "display中文"
        });
        fs::rename(&f.root, &root).unwrap();
        f.root = root;
        let source = ".secrets/test/key with space.enc";
        let workspace = f.root.join("shine.workspace.toml");
        fs::write(&workspace, format!("version=3\n[env]\ndefault_mode='test'\nmodes=['test']\n[env.encryption]\nbackend='gpg'\ngpg_recipients=['test']\n[[file_groups.signing.files]]\nsource={source:?}\ntarget={target:?}\n")).unwrap();
        let bytes = [0, 255, 10, 13, 128];
        fs::write(f.root.join(target), bytes).unwrap();
        let config = fs::read(f.root.join("state/config.toml")).unwrap();
        let workspace_bytes = fs::read(&workspace).unwrap();
        let root = fs::canonicalize(&f.root).unwrap();
        let from = format!("{:?}", root.join(source).to_string_lossy());
        let to = format!("{:?}", root.join(target).to_string_lossy());
        let log = f.root.join("backend-log");
        let seal = f
            .command()
            .env("PROBE_BACKEND_LOG", &log)
            .args([
                "env",
                "secret",
                "seal",
                "--file-group",
                "signing",
                "--dry-run",
            ])
            .output()
            .unwrap();
        assert!(seal.status.success(), "{seal:?}");
        let text = String::from_utf8(seal.stdout).unwrap();
        assert_eq!(
            text.lines().collect::<Vec<_>>(),
            [
                "File groups for mode test (metadata only)",
                &format!("Would seal {to} -> {from}")
            ]
        );
        assert!(seal.stderr.is_empty());
        assert!(!log.exists());
        assert!(!f.root.join(source).exists());
        assert_eq!(fs::read(f.root.join(target)).unwrap(), bytes);
        let sealed = f
            .command()
            .args([
                "env",
                "secret",
                "seal",
                "--file-group",
                "signing",
                "--remove-plaintext",
            ])
            .output()
            .unwrap();
        assert!(sealed.status.success(), "{sealed:?}");
        let text = String::from_utf8(sealed.stdout).unwrap();
        assert_eq!(text.lines().count(), 4, "{text:?}");
        assert!(text.lines().nth(2).unwrap().starts_with("Sealed "));
        assert!(
            text.lines()
                .last()
                .unwrap()
                .starts_with("Removed plaintext ")
        );
        assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
        assert!(!f.root.join(target).exists());
        for (args, verb) in [
            (
                vec![
                    "env",
                    "run",
                    "--file-group",
                    "signing",
                    "--dry-run",
                    "--",
                    "nonexistent-command",
                ],
                "decrypt temporarily",
            ),
            (
                vec![
                    "env",
                    "workspace",
                    "export",
                    "--format",
                    "files",
                    "--mode",
                    "test",
                    "--file-group",
                    "signing",
                    "--dry-run",
                ],
                "export plaintext",
            ),
        ] {
            let preview = f
                .command()
                .env("PROBE_BACKEND_LOG", &log)
                .args(args)
                .output()
                .unwrap();
            assert!(preview.status.success(), "{preview:?}");
            assert_eq!(
                String::from_utf8(preview.stdout)
                    .unwrap()
                    .lines()
                    .collect::<Vec<_>>(),
                [
                    "File groups for mode test (metadata only)",
                    &format!("Would {verb} {from} -> {to}")
                ]
            );
            assert!(preview.stderr.is_empty());
            assert!(!log.exists());
            assert!(!f.root.join(target).exists());
        }
        let run = f
            .command()
            .args(["env", "run", "--file-group", "signing", "--"])
            .arg(PROBE_BINARY.0.join(exe("probe")))
            .arg("output")
            .env("PROBE_EXIT", "42")
            .output()
            .unwrap();
        assert_eq!(run.status.code(), Some(42), "{run:?}");
        assert_eq!(run.stdout, bytes);
        assert!(
            String::from_utf8_lossy(&run.stderr)
                .lines()
                .any(|line| line == format!("Would decrypt temporarily {from} -> {to}"))
        );
        assert!(!f.root.join(target).exists());
        let exported = f
            .command()
            .args([
                "env",
                "workspace",
                "export",
                "--format",
                "files",
                "--mode",
                "test",
                "--file-group",
                "signing",
            ])
            .output()
            .unwrap();
        assert!(exported.status.success(), "{exported:?}");
        let text = String::from_utf8(exported.stdout).unwrap();
        assert_eq!(text.lines().count(), 2, "{text:?}");
        assert!(text.lines().next().unwrap().starts_with("Exported "));
        assert_eq!(text.lines().last(), Some("Exported 1 files for mode test"));
        assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
        assert!(
            String::from_utf8(exported.stderr)
                .unwrap()
                .contains(&format!("Would export plaintext {from} -> {to}"))
        );
        assert_eq!(fs::read(f.root.join(target)).unwrap(), bytes);
        assert_eq!(fs::read(f.root.join("state/config.toml")).unwrap(), config);
        assert_eq!(fs::read(&workspace).unwrap(), workspace_bytes);
    }
}

#[test]
fn file_groups_verification_and_decryption_failures_preserve_plaintext() {
    let f = Fixture::new();
    let out = f
        .command()
        .env("PROBE_FAIL_DECRYPT", "1")
        .args([
            "env",
            "secret",
            "seal",
            "--file-group",
            "signing",
            "--file-group",
            "account",
            "--remove-plaintext",
        ])
        .output()
        .unwrap();
    assert!(!String::from_utf8_lossy(&out.stderr).contains("must-not-leak-plaintext"));
    error(out, "verification failed");
    assert!(f.root.join(".signing/key").exists());
    assert!(f.root.join("other/password").exists());
    f.prepare();
    error(
        f.run("check")
            .env("PROBE_FAIL_DECRYPT", "1")
            .output()
            .unwrap(),
        "file decryption failed",
    );
    assert!(!f.root.join(".signing").exists());
}

#[test]
fn file_groups_accept_targets_removed_by_child_and_keep_exit_status() {
    for mode in ["remove", "remove-parent"] {
        for code in [0, 42] {
            let f = Fixture::new();
            f.prepare();
            let output = f
                .run(mode)
                .env("PROBE_EXIT", code.to_string())
                .output()
                .unwrap();
            assert_eq!(
                output.status.code(),
                Some(code),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(!String::from_utf8_lossy(&output.stderr).contains("cleanup failed"));
            assert!(!f.root.join(".signing").exists());
            // A child-removed target must not prevent cleanup of another mapping.
            assert!(!f.root.join("other").exists());
        }
    }
}

#[test]
fn file_groups_preserve_replacement_directories_during_cleanup() {
    let f = Fixture::new();
    f.prepare();
    error(f.run("replace-directory").output().unwrap(), "cleanup");
    assert!(f.root.join(".signing/key").is_dir());
    assert!(!f.root.join("other").exists());
}

#[cfg(unix)]
#[test]
fn file_groups_preserve_dangling_replacement_links_during_cleanup() {
    let f = Fixture::new();
    f.prepare();
    error(f.run("replace-dangling-link").output().unwrap(), "cleanup");
    assert!(
        fs::symlink_metadata(f.root.join(".signing/key"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(!f.root.join(".signing/missing").exists());
    assert!(!f.root.join("other").exists());
}

#[test]
fn file_groups_refuse_existing_targets_and_preserve_replacements() {
    let f = Fixture::new();
    f.prepare();
    fs::create_dir(f.root.join(".signing")).unwrap();
    fs::write(f.root.join(".signing/key"), "existing").unwrap();
    error(f.run("check").output().unwrap(), "already exists");
    assert_eq!(fs::read(f.root.join(".signing/key")).unwrap(), b"existing");
    fs::remove_file(f.root.join(".signing/key")).unwrap();
    error(f.run("replace").output().unwrap(), "cleanup");
    assert_eq!(
        fs::read(f.root.join(".signing/key")).unwrap(),
        b"replacement"
    );
    assert!(!f.root.join("other").exists());
}

#[test]
fn file_groups_reject_workspace_case_aliases_before_backend_or_deletion() {
    for (from, to) in [
        ("target='.signing/key'", "target='SHINE.WORKSPACE.TOML'"),
        (
            "source='.secrets/{mode}/key.enc'",
            "source='SHINE.WORKSPACE.TOML'",
        ),
    ] {
        let f = Fixture::new();
        let path = f.root.join("shine.workspace.toml");
        let contents = fs::read_to_string(&path).unwrap().replace(from, to);
        fs::write(&path, &contents).unwrap();
        let log = f.root.join("backend-called");
        for dry_run in [false, true] {
            let mut cmd = f.command();
            cmd.args([
                "env",
                "secret",
                "seal",
                "--file-group",
                "signing",
                "--remove-plaintext",
            ])
            .env("PROBE_BACKEND_LOG", &log);
            if dry_run {
                cmd.arg("--dry-run");
            }
            error(cmd.output().unwrap(), "mapping overlaps workspace file");
            assert_eq!(fs::read_to_string(&path).unwrap(), contents);
            assert!(!log.exists());
            assert!(!f.root.join(".secrets").exists());
        }
    }
}

#[test]
fn file_groups_reject_unicode_workspace_aliases_before_backend_or_deletion() {
    for (name, alias) in [
        ("unicode-\u{e9}.toml", "unicode-e\u{301}.toml"),
        ("unicode-e\u{301}.toml", "unicode-\u{e9}.toml"),
    ] {
        for field in ["source", "target"] {
            let f = Fixture::new();
            let path = f.root.join(name);
            let original = fs::read_to_string(f.root.join("shine.workspace.toml")).unwrap();
            let from = if field == "source" {
                "source='.secrets/{mode}/key.enc'"
            } else {
                "target='.signing/key'"
            };
            let contents = original.replace(from, &format!("{field}='{alias}'"));
            fs::write(&path, &contents).unwrap();
            let log = f.root.join("backend-called");
            for dry_run in [false, true] {
                let mut cmd = f.command();
                cmd.args([
                    "env",
                    "secret",
                    "seal",
                    "--file-group",
                    "signing",
                    "--remove-plaintext",
                    "--workspace",
                ])
                .arg(&path)
                .env("PROBE_BACKEND_LOG", &log);
                if dry_run {
                    cmd.arg("--dry-run");
                }
                error(cmd.output().unwrap(), "mapping overlaps workspace file");
                let mut export = f.export();
                export
                    .arg("--workspace")
                    .arg(&path)
                    .env("PROBE_BACKEND_LOG", &log);
                if dry_run {
                    export.arg("--dry-run");
                }
                error(export.output().unwrap(), "mapping overlaps workspace file");
                assert_eq!(fs::read_to_string(&path).unwrap(), contents);
                assert!(!log.exists());
                assert!(!f.root.join(".secrets").exists());
            }
        }
    }
}

#[test]
fn file_groups_reject_unsafe_paths_modes_versions_and_missing_groups() {
    for (from, to, expected) in [
        ("target='.signing/key'", "target='../outside'", "relative"),
        ("target='.signing/key'", "target='/outside'", "relative"),
        (
            "target='other/password'",
            "target='.signing/key'",
            "overlap",
        ),
        (
            "source='.secrets/{mode}/key.enc'",
            "source='.signing/key'",
            "overlap",
        ),
        ("version=3", "version=2", "file_groups requires"),
    ] {
        let f = Fixture::new();
        let path = f.root.join("shine.workspace.toml");
        fs::write(&path, fs::read_to_string(&path).unwrap().replace(from, to)).unwrap();
        error(f.seal(&[]), expected);
        error(f.export().output().unwrap(), expected);
    }
    let f = Fixture::new();
    error(f.seal(&["--mode", "unknown"]), "undeclared");
    error(f.seal(&["--file-group", "missing"]), "unknown file group");
    error(
        f.command()
            .args(["env", "run", "--", "true"])
            .output()
            .unwrap(),
        "requires --file-group",
    );
    error(
        f.seal(&["--file-group", "signing", "--no-such-option"]),
        "unexpected argument",
    );
}

#[test]
fn file_groups_require_git_ignored_untracked_plaintext() {
    let f = Fixture::new();
    good(
        Command::new("git")
            .current_dir(&f.root)
            .arg("init")
            .output()
            .unwrap(),
    );
    error(f.seal(&[]), "must be ignored");
    fs::write(f.root.join(".gitignore"), ".signing/\nother/\n").unwrap();
    good(f.seal(&[]));
    good(
        Command::new("git")
            .current_dir(&f.root)
            .args(["add", "--force", ".signing/key"])
            .output()
            .unwrap(),
    );
    error(f.seal(&[]), "is tracked");
}

fn fixture_git(root: &std::path::Path, args: &[&str]) {
    let mut command = Command::new("git");
    command.current_dir(root).args(args);
    for (key, _) in std::env::vars_os().filter(|(key, _)| key.to_string_lossy().starts_with("GIT_"))
    {
        command.env_remove(key);
    }
    good(command.output().unwrap());
}

#[test]
fn file_groups_reject_tracked_case_and_unicode_aliases_before_decryption() {
    for (tracked, target) in [
        (".signing/KEY", ".signing/key"),
        (".SIGNING/key", ".signing/key"),
        (".signing/k\u{e9}y", ".signing/ke\u{301}y"),
        (".signing/ke\u{301}y", ".signing/k\u{e9}y"),
    ] {
        let f = Fixture::new();
        f.prepare();
        fixture_git(&f.root, &["init"]);
        // Keep the exact Unicode spelling in the index on macOS too.
        fixture_git(&f.root, &["config", "core.precomposeunicode", "false"]);
        fs::write(f.root.join(".gitignore"), ".signing/\nother/\n").unwrap();
        fs::create_dir_all(f.root.join(tracked).parent().unwrap()).unwrap();
        fs::write(f.root.join(tracked), b"tracked fixture").unwrap();
        fixture_git(&f.root, &["add", "--force", "--", tracked]);
        fs::remove_file(f.root.join(tracked)).unwrap();
        // Existing parents exercise Git commands launched below the repository root.
        fs::create_dir_all(f.root.join(target).parent().unwrap()).unwrap();
        let workspace = f.root.join("shine.workspace.toml");
        fs::write(
            &workspace,
            fs::read_to_string(&workspace)
                .unwrap()
                .replace("target='.signing/key'", &format!("target='{target}'")),
        )
        .unwrap();
        let log = f.root.join("backend-log");
        for dry_run in [false, true] {
            let mut command = f.command();
            command.args(["env", "run", "--file-group", "signing"]);
            if dry_run {
                command.arg("--dry-run");
            }
            error(
                command
                    .args(["--", "unused-command"])
                    .env("PROBE_BACKEND_LOG", &log)
                    .output()
                    .unwrap(),
                "is tracked",
            );
            let mut export = f.export();
            if dry_run {
                export.arg("--dry-run");
            }
            error(
                export.env("PROBE_BACKEND_LOG", &log).output().unwrap(),
                "is tracked",
            );
            assert!(!log.exists());
            assert!(!f.root.join(target).exists());
        }
        fs::write(f.root.join(target), b"pending plaintext").unwrap();
        error(f.seal(&[]), "is tracked");
        assert_eq!(fs::read(f.root.join(target)).unwrap(), b"pending plaintext");
    }
}

#[test]
fn file_groups_check_nested_git_targets_even_outside_a_parent_repository() {
    let f = Fixture::new();
    f.prepare();
    // The signing target's parent is absent: discover from its closest existing ancestor.
    fs::create_dir(f.root.join("other")).unwrap();
    fixture_git(&f.root.join("other"), &["init"]);
    let log = f.root.join("backend-log");
    error(
        f.run("check")
            .env("PROBE_BACKEND_LOG", &log)
            .output()
            .unwrap(),
        "must be ignored",
    );
    assert!(!log.exists());
    assert!(!f.root.join(".signing/key").exists());
    fs::write(f.root.join("other/.gitignore"), "password\n").unwrap();
    good(f.run("check").output().unwrap());
    // An outer ignore rule must not hide the nested repository's own policy.
    fixture_git(&f.root, &["init"]);
    fs::write(f.root.join(".gitignore"), ".signing/\nother/\n").unwrap();
    fs::remove_file(f.root.join("other/.gitignore")).unwrap();
    error(f.run("check").output().unwrap(), "must be ignored");
    fs::write(f.root.join("other/.gitignore"), "password\n").unwrap();
    good(f.run("check").output().unwrap());
    fs::write(f.root.join("other/password"), "tracked fixture").unwrap();
    fixture_git(&f.root.join("other"), &["add", "--force", "password"]);
    fs::remove_file(f.root.join("other/password")).unwrap();
    error(f.run("check").output().unwrap(), "is tracked");
    assert!(!f.root.join("other/password").exists());
    // Sealing must also reject a non-ignored target in the nested repository.
    fs::write(f.root.join("other/password"), []).unwrap();
    error(f.seal(&[]), "is tracked");
}

#[test]
fn file_groups_stop_on_git_discovery_errors_before_decrypting() {
    let f = Fixture::new();
    f.prepare();
    fixture_git(&f.root, &["init"]);
    fs::write(f.root.join(".git/config"), "[broken\n").unwrap();
    let log = f.root.join("backend-log");
    error(
        f.run("check")
            .env("PROBE_BACKEND_LOG", &log)
            .output()
            .unwrap(),
        "cannot inspect Git repository",
    );
    assert!(!log.exists());
    assert!(!f.root.join(".signing/key").exists());
    assert!(!f.root.join("other/password").exists());
    fs::create_dir(f.root.join(".signing")).unwrap();
    fs::create_dir(f.root.join("other")).unwrap();
    fs::write(f.root.join(".signing/key"), [0, 255, 10, 13, 128]).unwrap();
    fs::write(f.root.join("other/password"), []).unwrap();
    error(f.seal(&[]), "cannot inspect Git repository");
    assert_eq!(
        fs::read(f.root.join(".signing/key")).unwrap(),
        [0, 255, 10, 13, 128]
    );
}

#[test]
fn file_groups_preserve_exact_child_stdout_and_keep_preview_on_stderr() {
    let f = Fixture::new();
    f.prepare();
    let output = f.run("output").output().unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, [0, 255, 10, 13, 128]);
    assert!(String::from_utf8_lossy(&output.stderr).contains("Would decrypt temporarily"));
    assert!(!f.root.join(".signing/key").exists());
    let preview = f
        .command()
        .args([
            "env",
            "run",
            "--file-group",
            "signing",
            "--dry-run",
            "--",
            "nonexistent-command",
        ])
        .output()
        .unwrap();
    assert!(preview.status.success());
    assert!(String::from_utf8_lossy(&preview.stdout).contains("Would decrypt temporarily"));
    assert!(!String::from_utf8_lossy(&preview.stderr).contains("Would decrypt temporarily"));
}

#[cfg(unix)]
#[test]
fn file_groups_reject_links_and_create_private_files() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let f = Fixture::new();
    fs::hard_link(f.root.join(".signing/key"), f.root.join("alias")).unwrap();
    error(f.seal(&[]), "hard link");
    fs::remove_file(f.root.join("alias")).unwrap();
    fs::rename(f.root.join(".signing"), f.root.join("real-signing")).unwrap();
    symlink("real-signing", f.root.join(".signing")).unwrap();
    error(f.seal(&[]), "linked file-group path");
    fs::remove_file(f.root.join(".signing")).unwrap();
    fs::rename(f.root.join("real-signing"), f.root.join(".signing")).unwrap();
    f.prepare();
    let mut child = f
        .run("wait")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    wait_for(&f.root.join("ready"));
    assert_eq!(
        fs::metadata(f.root.join(".signing/key"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(f.root.join(".signing"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
    }
    assert!(!child.wait().unwrap().success());
    assert!(!f.root.join(".signing").exists());
}

#[cfg(unix)]
#[test]
fn file_groups_serialize_conflicting_runs_and_clean_on_interrupt() {
    let f = Fixture::new();
    f.prepare();
    let child = f
        .run("wait")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    wait_for(&f.root.join("ready"));
    error(f.run("check").output().unwrap(), "already exists");
    // Seal shares locks with run even though it reads existing targets.
    error(f.seal(&[]), "another Shine file-group operation");
    unsafe {
        libc::kill(child.id() as i32, libc::SIGINT);
    }
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(130));
    assert!(!f.root.join(".signing").exists());
    assert!(!f.root.join("other").exists());
    good(f.run("check").output().unwrap());
}

#[cfg(unix)]
fn wait_for(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !path.exists() {
        assert!(Instant::now() < deadline, "probe did not become ready");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn file_groups_recheck_snapshots_and_reject_oversized_or_non_ciphertext_inputs() {
    let f = Fixture::new();
    let log = f.root.join("backend-log");
    let out = f
        .command()
        .env("PROBE_MUTATE", "shine.workspace.toml")
        .args(["env", "secret", "seal", "--file-group", "signing"])
        .output()
        .unwrap();
    error(out, "workspace changed");
    assert!(!f.root.join(".secrets").exists());
    good(f.seal(&[]));
    fs::write(f.root.join(".secrets/test/key.enc"), "unrelated file").unwrap();
    let out = f
        .command()
        .env("PROBE_BACKEND_LOG", &log)
        .args(["env", "secret", "seal", "--file-group", "signing"])
        .output()
        .unwrap();
    error(out, "ciphertext");
    assert!(!log.exists());
    fs::remove_dir_all(f.root.join(".secrets")).unwrap();
    let file = fs::OpenOptions::new()
        .write(true)
        .open(f.root.join(".signing/key"))
        .unwrap();
    file.set_len(16 * 1024 * 1024 + 1).unwrap();
    error(f.seal(&[]), "size limit");
    assert!(!f.root.join(".secrets").exists());
}

#[test]
fn file_groups_cli_rejects_conflicting_or_unscoped_flags() {
    let f = Fixture::new();
    for args in [
        vec![
            "env",
            "secret",
            "seal",
            "source.toml",
            "--file-group",
            "signing",
        ],
        vec!["env", "secret", "seal", "--remove-plaintext"],
        vec!["env", "secret", "seal", "--dry-run"],
        vec![
            "env",
            "run",
            "--file-group",
            "signing",
            "--no-workspace",
            "--",
            "true",
        ],
        vec![
            "env",
            "run",
            "--file-group",
            "signing",
            "--secret-broker",
            "--",
            "true",
        ],
        vec!["env", "run", "--dry-run", "--", "true"],
    ] {
        assert!(!f.command().args(args).output().unwrap().status.success());
    }
    good(f.seal(&["--remove-plaintext", "--dry-run"]));
    assert!(f.root.join(".signing/key").exists());
}

#[cfg(unix)]
#[test]
fn file_groups_roll_back_partial_materialization_and_keep_failed_child_status() {
    use std::os::unix::fs::PermissionsExt;
    let mut f = Fixture::new();
    f.prepare();
    fs::create_dir(f.root.join("other")).unwrap();
    fs::set_permissions(f.root.join("other"), fs::Permissions::from_mode(0o500)).unwrap();
    let output = f.run("check").output().unwrap();
    fs::set_permissions(f.root.join("other"), fs::Permissions::from_mode(0o700)).unwrap();
    assert!(!output.status.success());
    assert!(!f.root.join(".signing").exists());
    assert!(!f.root.join("other/password").exists());
    let root = f.root.with_extension("display\nFORGED=path\r\t\u{1b}中文");
    fs::rename(&f.root, &root).unwrap();
    f.root = root;
    let output = f.run("replace").env("PROBE_EXIT", "42").output().unwrap();
    assert_eq!(output.status.code(), Some(42));
    assert!(output.stdout.is_empty(), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("cleanup failed"));
    assert!(!stderr.contains(['\r', '\t', '\u{1b}']), "{stderr:?}");
    assert!(
        !stderr.lines().any(|line| line.starts_with("FORGED=")),
        "{stderr:?}"
    );
    assert_eq!(
        fs::read(f.root.join(".signing/key")).unwrap(),
        b"replacement"
    );
}

#[test]
fn file_groups_recheck_targets_before_plaintext_deletion() {
    let f = Fixture::new();
    let output = f
        .command()
        .env("PROBE_MUTATE", ".signing/key")
        .args([
            "env",
            "secret",
            "seal",
            "--file-group",
            "signing",
            "--file-group",
            "account",
            "--remove-plaintext",
        ])
        .output()
        .unwrap();
    error(output, "file changed");
    assert!(f.root.join(".signing/key").exists());
    assert!(f.root.join("other/password").exists());
    assert!(!f.root.join(".secrets").exists());
}

#[test]
fn file_export_persists_binary_and_empty_files_without_changing_inputs() {
    for mode in ["test", "prod"] {
        let f = Fixture::new();
        good(f.seal(&["--mode", mode, "--remove-plaintext"]));
        let workspace = fs::read(f.root.join("shine.workspace.toml")).unwrap();
        let source = f.root.join(format!(".secrets/{mode}/key.enc"));
        let cipher = fs::read(&source).unwrap();
        let output = f
            .command()
            .args([
                "env",
                "workspace",
                "export",
                "--format",
                "files",
                "--mode",
                mode,
                "--file-group",
                "signing",
                "--file-group",
                "account",
                "--file-group",
                "signing",
            ])
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&output.stdout).contains("Exported 2 files"));
        assert!(String::from_utf8_lossy(&output.stderr).contains("remain on disk"));
        good(output);
        assert_eq!(
            fs::read(f.root.join(".signing/key")).unwrap(),
            [0, 255, 10, 13, 128]
        );
        assert!(fs::read(f.root.join("other/password")).unwrap().is_empty());
        assert_eq!(fs::read(source).unwrap(), cipher);
        assert_eq!(
            fs::read(f.root.join("shine.workspace.toml")).unwrap(),
            workspace
        );
        error(
            f.command()
                .args([
                    "env",
                    "run",
                    "--mode",
                    mode,
                    "--file-group",
                    "signing",
                    "--",
                    "never-started",
                ])
                .output()
                .unwrap(),
            "plaintext target already exists",
        );
        error(
            f.command()
                .args([
                    "env",
                    "workspace",
                    "export",
                    "--format",
                    "files",
                    "--mode",
                    mode,
                    "--file-group",
                    "signing",
                ])
                .output()
                .unwrap(),
            "plaintext target already exists",
        );
    }
}

#[test]
fn file_export_preview_and_preflight_never_decrypt_or_compile_env_sources() {
    let f = Fixture::new();
    f.prepare();
    let workspace = f.root.join("shine.workspace.toml");
    let text = fs::read_to_string(&workspace)
        .unwrap()
        .replace("[env]", "[env]\nfiles=['missing-env-source.toml']");
    fs::write(&workspace, text).unwrap();
    let log = f.root.join("backend-log");
    let output = f
        .export()
        .arg("--dry-run")
        .env("PROBE_BACKEND_LOG", &log)
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&output.stdout).contains("Would export plaintext"));
    good(output);
    assert!(!log.exists());
    assert!(!f.root.join(".signing").exists());
    good(f.export().output().unwrap());
    assert!(f.root.join(".signing/key").exists());
}

#[test]
fn file_export_invalid_selection_and_existing_targets_fail_before_backend() {
    let f = Fixture::new();
    f.prepare();
    let log = f.root.join("backend-log");
    error(
        f.export()
            .args(["--file-group", "unknown"])
            .env("PROBE_BACKEND_LOG", &log)
            .output()
            .unwrap(),
        "unknown file group",
    );
    let output = f
        .command()
        .args([
            "env",
            "workspace",
            "export",
            "--format",
            "files",
            "--mode",
            "missing",
            "--file-group",
            "signing",
        ])
        .env("PROBE_BACKEND_LOG", &log)
        .output()
        .unwrap();
    error(output, "undeclared file-group mode");
    fs::create_dir(f.root.join("other")).unwrap();
    fs::write(f.root.join("other/password"), "existing").unwrap();
    error(
        f.export().env("PROBE_BACKEND_LOG", &log).output().unwrap(),
        "plaintext target already exists",
    );
    assert!(!log.exists());
    assert!(!f.root.join(".signing").exists());
    assert_eq!(
        fs::read(f.root.join("other/password")).unwrap(),
        b"existing"
    );
}

#[test]
fn file_export_decrypt_failure_and_snapshot_changes_leave_no_plaintext() {
    for mutation in ["decrypt", "second-decrypt", "workspace", "ciphertext"] {
        let f = Fixture::new();
        f.prepare();
        let mut cmd = f.export();
        let expected = match mutation {
            "decrypt" => {
                cmd.env("PROBE_FAIL_DECRYPT", "1");
                "file decryption failed"
            }
            "second-decrypt" => {
                cmd.env("PROBE_FAIL_SECOND", f.root.join("counter"));
                "file decryption failed"
            }
            "workspace" => {
                cmd.env("PROBE_MUTATE", "shine.workspace.toml");
                "workspace changed"
            }
            _ => {
                cmd.env("PROBE_MUTATE", ".secrets/test/key.enc");
                "file changed during operation"
            }
        };
        let output = cmd.output().unwrap();
        assert!(!String::from_utf8_lossy(&output.stderr).contains("must-not-leak-plaintext"));
        error(output, expected);
        assert!(!f.root.join(".signing").exists());
        assert!(!f.root.join("other").exists());
    }
}

#[test]
fn file_export_rechecks_late_targets_and_shares_locks_across_config_scopes() {
    let f = Fixture::new();
    f.prepare();
    let mut first = f.blocked_export();
    f.await_export(&mut first);
    let second_state = f.root.join("second-state");
    fs::create_dir(&second_state).unwrap();
    fs::write(second_state.join("config.toml"), "schema_version=2\n").unwrap();
    error(
        f.export()
            .env("SHINE_CONFIG_DIR", &second_state)
            .output()
            .unwrap(),
        "another Shine file-group operation is active",
    );
    error(
        f.run("check").output().unwrap(),
        "another Shine file-group operation is active",
    );
    fs::create_dir(f.root.join("other")).unwrap();
    fs::write(f.root.join("other/password"), "late winner").unwrap();
    f.release_export();
    error(
        first.wait_with_output().unwrap(),
        "plaintext target already exists",
    );
    assert!(!f.root.join(".signing").exists());
    assert_eq!(
        fs::read(f.root.join("other/password")).unwrap(),
        b"late winner"
    );
}

#[test]
fn file_export_git_policy_follows_nested_targets_and_fails_closed() {
    for policy in ["ignored", "unignored", "tracked", "broken"] {
        let f = Fixture::new();
        f.prepare();
        fixture_git(&f.root, &["init"]);
        fs::write(f.root.join(".gitignore"), ".signing/\nother/\n").unwrap();
        fs::create_dir(f.root.join("other")).unwrap();
        fixture_git(&f.root.join("other"), &["init"]);
        if policy == "ignored" {
            fs::write(f.root.join("other/.gitignore"), "password\n").unwrap();
        }
        if policy == "tracked" {
            fs::write(f.root.join("other/password"), "tracked").unwrap();
            fixture_git(&f.root.join("other"), &["add", "password"]);
            fs::remove_file(f.root.join("other/password")).unwrap();
            fs::write(f.root.join("other/.gitignore"), "password\n").unwrap();
        }
        if policy == "broken" {
            fs::write(f.root.join("other/.git/config"), "[broken\n").unwrap();
        }
        let log = f.root.join("backend-log");
        let output = f.export().env("PROBE_BACKEND_LOG", &log).output().unwrap();
        if policy == "ignored" {
            good(output);
        } else {
            error(
                output,
                match policy {
                    "tracked" => "is tracked",
                    "broken" => "cannot inspect Git repository",
                    _ => "must be ignored",
                },
            );
            assert!(!log.exists());
            assert!(!f.root.join(".signing").exists());
            assert!(!f.root.join("other/password").exists());
        }
    }
}

#[cfg(unix)]
#[test]
fn file_export_uses_private_permissions_and_rejects_links() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let f = Fixture::new();
    f.prepare();
    symlink("missing", f.root.join("other")).unwrap();
    error(f.export().output().unwrap(), "linked file-group path");
    fs::remove_file(f.root.join("other")).unwrap();
    good(f.export().output().unwrap());
    for target in [".signing/key", "other/password"] {
        assert_eq!(
            fs::metadata(f.root.join(target))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    for directory in [".signing", "other"] {
        assert_eq!(
            fs::metadata(f.root.join(directory))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
}

#[cfg(unix)]
#[test]
fn file_export_interrupt_during_authorization_releases_locks() {
    let f = Fixture::new();
    f.prepare();
    let mut child = f.blocked_export();
    f.await_export(&mut child);
    unsafe {
        libc::kill(child.id() as i32, libc::SIGINT);
    }
    error(
        child.wait_with_output().unwrap(),
        "file-group preparation interrupted",
    );
    assert!(!f.root.join(".signing").exists());
    assert!(!f.root.join("other").exists());
    good(f.export().output().unwrap());
}

#[test]
fn file_export_age_preserves_binary_files_and_phone_authorization() {
    use base64::{Engine, engine::general_purpose::STANDARD as B64};
    let f = Fixture::new();
    f.prepare();
    let identity = f.root.join("identity.txt");
    fs::write(&identity, "AGE-PLUGIN-PHONE-1EXAMPLE\n").unwrap();
    let mut config: toml::Table = toml::from_str("schema_version=2").unwrap();
    config.insert(
        "age_identity".into(),
        identity.to_string_lossy().as_ref().into(),
    );
    fs::write(
        f.root.join("state/config.toml"),
        toml::to_string(&config).unwrap(),
    )
    .unwrap();
    for (name, payload) in [("key", &b"fixture"[..]), ("password", &b"empty"[..])] {
        let bytes = [b"age-encryption.org/v1\n".as_slice(), payload].concat();
        let cipher = format!("age:{}", B64.encode(bytes));
        fs::write(f.root.join(format!(".secrets/test/{name}.enc")), &cipher).unwrap();
    }
    let output = f
        .export()
        .env("AGE_PLUGIN_PHONE_TRANSPORT", "qr")
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&output.stderr).contains("phone-authorization-prompt"));
    assert!(!output.stdout.contains(&255));
    good(output);
    assert_eq!(
        fs::read(f.root.join(".signing/key")).unwrap(),
        [0, 255, 10, 13, 128]
    );
    assert!(fs::read(f.root.join("other/password")).unwrap().is_empty());
}

// Frozen hybrid wire fixture: the stand-in GPG wrapper transports the data key;
// the real in-process AEAD still authenticates the envelope and binary plaintext.
fn hybrid_file_fixture(plaintext: &[u8]) -> String {
    use base64::{Engine, engine::general_purpose::STANDARD as B64};
    use chacha20poly1305::{
        KeyInit, XChaCha20Poly1305, XNonce,
        aead::{Aead, Payload},
    };
    let nonce = [9u8; 24];
    let key = [7u8; 32];
    let material: Vec<u8> = nonce.into_iter().chain(key).collect();
    let wrapped = format!("shine-hybrid-key-v1:{}", B64.encode(material));
    let mut gpg = vec![0xc1, 10, 3, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0xd2, 0xff];
    gpg.extend_from_slice(&((wrapped.len() + 1) as u32).to_be_bytes());
    gpg.push(1);
    gpg.extend_from_slice(wrapped.as_bytes());
    let age = b"age-encryption.org/v1\nfixture";
    let mut bytes = vec![1, 1];
    bytes.extend_from_slice(&nonce);
    for length in [gpg.len(), age.len(), plaintext.len() + 16] {
        bytes.extend_from_slice(&(length as u32).to_be_bytes());
    }
    bytes.extend_from_slice(&gpg);
    bytes.extend_from_slice(age);
    let cipher = XChaCha20Poly1305::new_from_slice(&key).unwrap();
    let data = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad: &bytes,
            },
        )
        .unwrap();
    bytes.extend_from_slice(&data);
    format!("hybrid:{}", B64.encode(bytes))
}

#[test]
fn file_export_hybrid_honors_local_preference_and_never_falls_back_on_failure() {
    for fail in [false, true] {
        let f = Fixture::new();
        f.prepare();
        fs::write(
            f.root.join("state/config.toml"),
            "schema_version=2\nhybrid_decrypt_backend='age'\n",
        )
        .unwrap();
        fs::write(
            f.root.join("shine.config.local.toml"),
            "hybrid_decrypt_backend='gpg'\n",
        )
        .unwrap();
        for (name, plain) in [("key", &[0, 255, 10, 13, 128][..]), ("password", &[][..])] {
            fs::write(
                f.root.join(format!(".secrets/test/{name}.enc")),
                hybrid_file_fixture(plain),
            )
            .unwrap();
        }
        let mut cmd = f.export();
        if fail {
            cmd.env("PROBE_FAIL_DECRYPT", "1");
        }
        let output = cmd.output().unwrap();
        assert!(!String::from_utf8_lossy(&output.stderr).contains("must-not-leak-plaintext"));
        if fail {
            error(output, "file decryption failed");
            assert!(!f.root.join(".signing").exists());
            assert!(!f.root.join("other").exists());
        } else {
            good(output);
            assert_eq!(
                fs::read(f.root.join(".signing/key")).unwrap(),
                [0, 255, 10, 13, 128]
            );
            assert!(fs::read(f.root.join("other/password")).unwrap().is_empty());
        }
    }
}

#[test]
fn file_export_concurrent_invocations_have_one_successful_owner() {
    let f = Fixture::new();
    f.prepare();
    let mut first = f.blocked_export();
    f.await_export(&mut first);
    error(
        f.export().output().unwrap(),
        "another Shine file-group operation is active",
    );
    f.release_export();
    good(first.wait_with_output().unwrap());
    error(
        f.export().output().unwrap(),
        "plaintext target already exists",
    );
    assert_eq!(
        fs::read(f.root.join(".signing/key")).unwrap(),
        [0, 255, 10, 13, 128]
    );
    assert!(fs::read(f.root.join("other/password")).unwrap().is_empty());
}
