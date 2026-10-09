//! Real launcher tests using an isolated installation and a locally compiled Bun probe.
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::LazyLock,
};

const PROBE: &str = r#"
use std::{env, fs, io::Write, process::Command};
fn main() {
    if env::current_exe().unwrap().file_stem().unwrap() == "shine" {
        let mut log = fs::OpenOptions::new().append(true).create(true).open(env::var_os("PROBE_SHINE_LOG").unwrap()).unwrap();
        writeln!(log, "shine").unwrap();
        let status = Command::new(env::var_os("PROBE_REAL_SHINE").unwrap()).args(env::args_os().skip(1)).status().unwrap();
        std::process::exit(status.code().unwrap_or(1));
    }
    println!("token={}", env::var("ALIAS").unwrap_or_default());
    println!("inherited={}", env::var("INHERITED").unwrap_or_default());
    println!("undeclared={}", env::var("UNDECLARED").unwrap_or_default());
    println!("cwd={}", env::current_dir().unwrap().display());
    let args: Vec<_> = env::args_os().skip(1).collect();
    println!("mode={}", args[0].to_string_lossy());
    println!("script={}", fs::read_to_string(&args[1]).unwrap().trim());
    for arg in &args[2..] {
        #[cfg(unix)] let bytes = { use std::os::unix::ffi::OsStrExt; arg.as_bytes().to_vec() };
        #[cfg(windows)] let bytes = arg.to_string_lossy().as_bytes().to_vec();
        println!("arg={}", bytes.iter().map(|byte| format!("{byte:02x}")).collect::<String>());
    }
    if env::var("PROBE_READ_STDIN").is_ok() {
        if let Some(ready) = env::var_os("PROBE_STDIN_READY") { fs::write(ready, "ready").unwrap(); }
        let mut input = String::new(); std::io::stdin().read_line(&mut input).unwrap(); println!("input={}", input.trim());
    }
    if let Some(ready) = env::var_os("PROBE_READY") {
        fs::write(ready, std::process::id().to_string()).unwrap();
        loop { std::thread::sleep(std::time::Duration::from_secs(1)); }
    }
    #[cfg(unix)] if env::var("PROBE_SIGNAL").is_ok() {
        Command::new("/bin/kill").args(["-TERM", &std::process::id().to_string()]).status().unwrap();
    }
    std::process::exit(env::var("PROBE_EXIT").unwrap_or_else(|_| "0".into()).parse().unwrap());
}
"#;

struct Probe(PathBuf);
impl Drop for Probe {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
static PROBE_BINARY: LazyLock<Probe> = LazyLock::new(|| {
    let dir = std::env::temp_dir().join(format!("shine-live-bun-probe-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("probe.rs"), PROBE).unwrap();
    let output = Command::new("rustc")
        .arg("--edition=2024")
        .arg(dir.join("probe.rs"))
        .arg("-o")
        .arg(dir.join(executable("probe")))
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output);
    Probe(dir)
});

fn executable(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}
struct Fixture {
    root: PathBuf,
    state: PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
impl Fixture {
    fn new(env: bool) -> Self {
        let root = std::env::temp_dir().join(format!("shine-live-bun-{}", uuid::Uuid::new_v4()));
        // This is a nondefault directory named .shine, with literal shell syntax in its parent.
        let state = root
            .join("state 'quotes' $literal `literal` %PROBE_EXPAND% !literal! &")
            .join(".shine");
        for dir in [
            state.clone(),
            root.join("tools"),
            root.join("other"),
            root.join("project"),
            root.join("home/AppData/Roaming"),
            root.join("home/AppData/Local"),
            state.join("presets/shell/demo"),
        ] {
            fs::create_dir_all(dir).unwrap();
        }
        let config = "schema_version = 2\nexternal_shell_mode = 'live'\n[env]\nTOKEN = 'installed'\nUNDECLARED = 'must-not-inject'\nVALUE = 'installed-template'\n";
        fs::write(state.join("config.toml"), config).unwrap();
        fs::write(
            root.join("other/config.toml"),
            config.replace("installed", "wrong-store"),
        )
        .unwrap();
        fs::write(
            root.join("project/shine.config.toml"),
            "[env]\nTOKEN = 'project'\nVALUE = 'project-template'\n",
        )
        .unwrap();
        fs::write(
            root.join("project/shine.env.toml"),
            "TOKEN = 'override'\nVALUE = 'override-template'\n",
        )
        .unwrap();
        fs::write(
            root.join("project/shine.workspace.toml"),
            "invalid workspace deliberately ignored",
        )
        .unwrap();
        fs::write(state.join("presets/shell/demo/run.ts"), "@@VALUE@@\n").unwrap();
        fs::write(state.join("presets/shell/demo/shine.toml"), format!("[permission_defaults]\nschema_version = 2\nopaque_code = 'unrestricted'\nenvironment = [{{ name = 'TOKEN', sensitivity = 'plain' }}, {{ name = 'VALUE', sensitivity = 'plain' }}]\n[[files]]\nsource = 'run.ts'\ntarget = 'run'\nruntime = 'bun'\ntransforms = ['template']\nenv = {}\n", if env { "['TOKEN=ALIAS']" } else { "[]" })).unwrap();
        for name in ["bun", "shine"] {
            fs::copy(
                PROBE_BINARY.0.join(executable("probe")),
                root.join("tools").join(executable(name)),
            )
            .unwrap();
        }
        let fixture = Self { root, state };
        fixture.assert_ok(
            fixture
                .shine()
                .args(["trust", "grant", "shell/demo/run", "--development", "--yes"])
                .output()
                .unwrap(),
        );
        fixture.assert_ok(
            fixture
                .shine()
                .args(["install", "shell/demo/run", "--yes"])
                .output()
                .unwrap(),
        );
        fixture
    }
    fn command(&self, program: &Path) -> Command {
        let mut command = Command::new(program);
        // Keep OS runtime variables (SystemRoot on Windows) while overriding all Shine roots.
        command
            .env("SHINE_CONFIG_DIR", self.root.join("other"))
            .env_remove("SHINE_PRESETS")
            .env("HOME", self.root.join("home"))
            .env("USERPROFILE", self.root.join("home"))
            .env("APPDATA", self.root.join("home/AppData/Roaming"))
            .env("LOCALAPPDATA", self.root.join("home/AppData/Local"))
            .env("PROBE_EXPAND", "incorrect-expanded-path")
            .env("PROBE_REAL_SHINE", env!("CARGO_BIN_EXE_shine"))
            .env("PROBE_SHINE_LOG", self.root.join("shine.log"))
            .env("INHERITED", "retained")
            .env_remove("UNDECLARED")
            .env("ALIAS", "inherited-wrong")
            .env(
                "PATH",
                std::env::join_paths(std::iter::once(self.root.join("tools")).chain(
                    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
                ))
                .unwrap(),
            )
            .current_dir(self.root.join("project"))
            .stdin(Stdio::null());
        command
    }
    fn shine(&self) -> Command {
        let mut command = self.command(Path::new(env!("CARGO_BIN_EXE_shine")));
        command
            .arg("--config-dir")
            .arg(&self.state)
            .current_dir(&self.root);
        command
    }
    fn launcher(&self) -> Command {
        #[cfg(unix)]
        {
            self.command(&self.state.join("bin/run"))
        }
        #[cfg(windows)]
        {
            let mut command = self.command(Path::new("cmd.exe"));
            let entry = self.root.join("entry.cmd");
            fs::copy(self.state.join("bin/run.cmd"), &entry).unwrap();
            command.args(["/D", "/V:ON", "/C"]).arg(entry);
            command
        }
    }
    fn assert_ok(&self, output: Output) -> String {
        assert!(output.status.success(), "{output:?}");
        String::from_utf8(output.stdout).unwrap()
    }
}

#[test]
fn live_bun_launcher_binds_state_keeps_project_layers_and_argv() {
    let fixture = Fixture::new(true);
    let output = fixture.assert_ok(
        fixture
            .launcher()
            .args(["", "--config-dir", "--", "中文 value", "--flag"])
            .output()
            .unwrap(),
    );
    assert!(output.contains("token=override"), "{output}");
    assert!(output.contains("script=override-template"), "{output}");
    assert!(output.contains("inherited=retained"));
    assert!(output.contains("undeclared=\n") || output.contains("undeclared=\r\n"));
    assert!(output.contains("arg=\n") || output.contains("arg=\r\n"));
    assert!(output.contains("arg=2d2d636f6e6669672d646972"));
    assert_eq!(
        fs::read_to_string(fixture.root.join("shine.log"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    let rendered = fixture.state.join("rendered/shell/demo/run.ts");
    let before = fs::metadata(&rendered).unwrap().modified().unwrap();
    fixture.assert_ok(fixture.launcher().output().unwrap());
    assert_eq!(fs::metadata(&rendered).unwrap().modified().unwrap(), before);
    fs::write(
        fixture.state.join("presets/shell/demo/run.ts"),
        "changed @@VALUE@@\n",
    )
    .unwrap();
    let output = fixture.assert_ok(fixture.launcher().output().unwrap());
    assert!(output.contains("script=changed override-template"));
    // Metadata cannot silently expand the installed env declaration or change the runtime.
    fs::write(
        fixture.state.join("presets/shell/demo/shine.toml"),
        "invalid changed metadata",
    )
    .unwrap();
    assert!(
        fixture
            .assert_ok(fixture.launcher().output().unwrap())
            .contains("token=override")
    );
}

#[test]
fn live_bun_empty_env_exit_codes_and_errors() {
    let fixture = Fixture::new(false);
    let output = fixture.assert_ok(fixture.launcher().output().unwrap());
    assert!(output.contains("token=inherited-wrong"));
    assert!(output.contains("script=override-template"));
    assert_eq!(
        fixture
            .launcher()
            .env("PROBE_EXIT", "42")
            .output()
            .unwrap()
            .status
            .code(),
        Some(42)
    );
    let rendered = fixture.state.join("rendered/shell/demo/run.ts");
    let last_good = fs::read(&rendered).unwrap();
    fs::write(
        fixture.state.join("presets/shell/demo/run.ts"),
        "@@MISSING@@",
    )
    .unwrap();
    let output = fixture.launcher().output().unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(fs::read(&rendered).unwrap(), last_good);
    fs::write(
        fixture.state.join("shell-operation-journal.toml"),
        "corrupt journal",
    )
    .unwrap();
    assert!(!fixture.launcher().output().unwrap().status.success());
    assert!(
        fixture
            .shine()
            .args(["__shell-launch", "shell/demo/run"])
            .output()
            .unwrap()
            .status
            .code()
            .is_some_and(|code| code != 0)
    );
}

#[cfg(unix)]
#[test]
fn live_bun_preserves_non_utf8_arguments_and_signal_exit_mapping() {
    use std::os::unix::ffi::OsStringExt;
    let fixture = Fixture::new(true);
    let output = fixture.assert_ok(
        fixture
            .launcher()
            .arg(std::ffi::OsString::from_vec(vec![0xff, b'x']))
            .output()
            .unwrap(),
    );
    assert!(output.contains("arg=ff78"));
    assert_eq!(
        fixture
            .launcher()
            .env("PROBE_SIGNAL", "1")
            .output()
            .unwrap()
            .status
            .code(),
        Some(143)
    );
}

#[test]
fn live_bun_missing_env_and_corrupt_secret_do_not_execute_or_leak() {
    let fixture = Fixture::new(true);
    fs::write(
        fixture.root.join("project/shine.env.toml"),
        "VALUE = 'latest-template'\nTOKEN_SECRET = 'age:DO_NOT_LOG_THIS_CIPHERTEXT'\n",
    )
    .unwrap();
    let output = fixture.launcher().output().unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("DO_NOT_LOG_THIS_CIPHERTEXT"));
    assert_eq!(
        fs::read_to_string(fixture.state.join("rendered/shell/demo/run.ts")).unwrap(),
        "latest-template\n"
    );
    fs::write(
        fixture.root.join("project/shine.env.toml"),
        "VALUE = 'latest-template'\n",
    )
    .unwrap();
    for path in [
        fixture.state.join("config.toml"),
        fixture.root.join("project/shine.config.toml"),
    ] {
        let content = fs::read_to_string(&path).unwrap();
        fs::write(
            path,
            content
                .lines()
                .filter(|line| !line.starts_with("TOKEN ="))
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
    }
    let output = fixture.launcher().output().unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}

#[cfg(unix)]
#[test]
fn live_bun_missing_binary_returns_127() {
    let fixture = Fixture::new(false);
    fs::remove_file(fixture.root.join("tools/bun")).unwrap();
    let output = fixture
        .launcher()
        .env(
            "PATH",
            std::env::join_paths([
                fixture.root.join("tools"),
                PathBuf::from("/usr/bin"),
                PathBuf::from("/bin"),
            ])
            .unwrap(),
        )
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(127));
}

#[cfg(unix)]
#[test]
fn live_bun_receipt_is_read_after_operation_lock_is_acquired() {
    use shine_core::runtime::{PrivilegedFileSystemHost, RealHost};
    let fixture = Fixture::new(true);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let guard = runtime
        .block_on(RealHost.acquire_privileged_operation())
        .unwrap();
    let mut child = fixture
        .launcher()
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(200));
    assert!(child.try_wait().unwrap().is_none());
    let path = fixture.state.join("shell-manifest.toml");
    let mut manifest: shine_core::runtime::ShellManifest =
        toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    manifest.entries[0].env.clear();
    fs::write(path, toml::to_string(&manifest).unwrap()).unwrap();
    drop(guard);
    let output = fixture.assert_ok(child.wait_with_output().unwrap());
    assert!(output.contains("token=inherited-wrong"), "{output}");
}

#[cfg(unix)]
#[test]
fn live_bun_parent_signal_is_relayed_and_child_is_reaped() {
    let fixture = Fixture::new(false);
    let ready = fixture.root.join("ready");
    let mut child = fixture
        .shine()
        .args(["__shell-launch", "shell/demo/run", "--"])
        .env("PROBE_READY", &ready)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !ready.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    if !ready.exists() {
        child.kill().unwrap();
        let _ = child.wait();
        panic!("Bun did not become ready");
    }
    let pid: i32 = fs::read_to_string(&ready).unwrap().parse().unwrap();
    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
    }
    let mut status = None;
    while std::time::Instant::now() < deadline {
        status = child.try_wait().unwrap();
        if status.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    if status.is_none() {
        unsafe {
            libc::kill(pid, libc::SIGKILL);
        }
        let _ = child.kill();
        let _ = child.wait();
    }
    let _ = child.wait(); // try_wait may already have reaped it; always complete child ownership.
    assert_eq!(status.and_then(|status| status.code()), Some(143));
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1, "Bun remained alive");
}

#[cfg(unix)]
#[test]
fn live_bun_foreground_group_interrupt_is_reaped() {
    let fixture = Fixture::new(false);
    let ready = fixture.root.join("ready");
    use std::os::unix::process::CommandExt;
    let mut command = fixture.shine();
    command.process_group(0);
    let mut child = command
        .args(["__shell-launch", "shell/demo/run", "--"])
        .env("PROBE_READY", &ready)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !ready.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    if !ready.exists() {
        child.kill().unwrap();
        let _ = child.wait();
        panic!("Bun did not become ready");
    }
    let pid: i32 = fs::read_to_string(&ready).unwrap().parse().unwrap();
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGINT);
    }
    let mut status = None;
    while std::time::Instant::now() < deadline {
        status = child.try_wait().unwrap();
        if status.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    if status.is_none() {
        unsafe {
            libc::kill(pid, libc::SIGKILL);
        }
        let _ = child.kill();
        let _ = child.wait();
    }
    let _ = child.wait(); // try_wait may already have reaped it; always complete child ownership.
    assert_eq!(status.and_then(|status| status.code()), Some(130));
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1, "Bun remained alive");
}

#[cfg(unix)]
#[test]
fn live_bun_accepts_inherited_terminal_input() {
    use std::{io::Write, os::fd::FromRawFd};
    let fixture = Fixture::new(false);
    let (mut master, mut slave) = (-1, -1);
    assert_eq!(
        unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        },
        0
    );
    let mut master = unsafe { fs::File::from_raw_fd(master) };
    let slave = unsafe { fs::File::from_raw_fd(slave) };
    let ready = fixture.root.join("stdin-ready");
    let mut command = fixture.shine();
    command
        .args(["__shell-launch", "shell/demo/run", "--"])
        .env("PROBE_READ_STDIN", "1")
        .env("PROBE_STDIN_READY", &ready)
        .stdin(slave)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !ready.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    if !ready.exists() {
        unsafe {
            libc::kill(child.id() as i32, libc::SIGKILL);
        }
        let _ = child.kill();
        let output = child.wait_with_output().unwrap();
        panic!("terminal child did not reach stdin: {output:?}");
    }
    master.write_all(b"terminal-input\n").unwrap();
    while child.try_wait().unwrap().is_none() {
        if std::time::Instant::now() >= deadline {
            unsafe {
                libc::kill(child.id() as i32, libc::SIGKILL);
            }
            let _ = child.kill();
            let output = child.wait_with_output().unwrap();
            panic!("terminal child did not read input: {output:?}");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let output = fixture.assert_ok(child.wait_with_output().unwrap());
    assert!(output.contains("input=terminal-input"), "{output}");
}

#[cfg(windows)]
#[test]
fn live_bun_powershell_launcher_executes_with_literal_state_path() {
    let fixture = Fixture::new(true);
    let mut command = fixture.command(Path::new("powershell.exe"));
    // Call the launcher as a script so PowerShell's own -File argument parser
    // cannot consume the empty argument or the literal `--` before our shim.
    let entry = fixture.root.join("entry.ps1");
    let path = fixture
        .state
        .join("bin/run.ps1")
        .display()
        .to_string()
        .replace('\'', "''");
    fs::write(&entry, format!("& '{path}' '' '--' 'hello' 'space value' 'quote\"value' 'trailing\\' 'slash\\\"quote'\nexit $LASTEXITCODE\n")).unwrap();
    command.args(["-NoProfile", "-File"]).arg(entry);
    let output = fixture.assert_ok(command.output().unwrap());
    assert!(output.contains("token=override"));
    for arg in [
        "",
        "--",
        "hello",
        "space value",
        "quote\"value",
        "trailing\\",
        "slash\\\"quote",
    ] {
        let hex = arg
            .as_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert!(
            output.lines().any(|line| line == format!("arg={hex}")),
            "missing {arg:?}: {output}"
        );
    }
}
