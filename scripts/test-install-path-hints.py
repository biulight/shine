"""Run with python3 scripts/test-install-path-hints.py; no downloads or real profiles."""

import io
import errno
import os
from pathlib import Path
import pty
import re
import shutil
import subprocess
import tarfile
import tempfile
import threading
import unittest


INSTALLER = Path(__file__).resolve().parents[1] / "install.sh"


class PathHints(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.home = self.root / "home"
        self.home.mkdir()
        self.bin = self.root / "bin"
        self.bin.mkdir()
        archive = self.root / "release.tar.gz"
        with tarfile.open(archive, "w:gz") as tar:
            data = b"#!/bin/sh\nprintf 'installed-binary'\n"
            entry = tarfile.TarInfo("shine")
            entry.size = len(data)
            tar.addfile(entry, io.BytesIO(data))
        curl = self.bin / "curl"
        curl.write_text('#!/bin/sh\ncp "$TEST_ARCHIVE" "$4"\n')
        curl.chmod(0o755)
        self.env = dict(os.environ)
        self.env.pop("ZDOTDIR", None)
        self.env.update(
            HOME=str(self.home),
            PATH=str(self.bin) + ":/usr/bin:/bin",
            SHINE_VERSION="0.0.0-test",
            TEST_ARCHIVE=str(archive),
        )

    def install(self, shell, directory):
        self.env["SHINE_INSTALL_DIR"] = str(directory)
        if shell is None:
            self.env.pop("SHELL", None)
        else:
            self.env["SHELL"] = shell
        # Some /bin/sh implementations initialize an absent SHELL themselves.
        argv = ["/bin/sh", str(INSTALLER)]
        if shell is None:
            argv = ["/bin/sh", "-c", 'unset SHELL; . "$1"', "test", str(INSTALLER)]
        output = subprocess.check_output(argv, env=self.env, text=True)
        self.assertTrue((directory / "shine").is_file())
        return output

    def test_bash_and_zsh_commands_preserve_paths_and_existing_config(self):
        for shell in ("bash", "zsh"):
            executable = shutil.which(shell)
            if not executable:
                continue
            for name in (
                "simple bin",
                "quote' dollar$ back` slash\\ double\" glob*?[x] bin",
            ):
                with self.subTest(shell=shell, name=name):
                    directory = self.root / name
                    output = self.install("/bin/" + shell, directory)
                    profile_dir = self.home if shell == "bash" else self.root / "z dir"
                    profile_dir.mkdir(exist_ok=True)
                    profile = profile_dir / (".bashrc" if shell == "bash" else ".zshrc")
                    profile.write_text("# keep without final newline")
                    command = next(
                        x[2:] for x in output.splitlines() if x.startswith("  ")
                    )
                    # ZDOTDIR is deliberately not exported to the installer.
                    setup = 'ZDOTDIR="$HOME/../z dir"; ' if shell == "zsh" else ""
                    for _ in range(2):
                        subprocess.run(
                            [executable, "-f", "-c", setup + command],
                            env=self.env,
                            check=True,
                        )
                    content = profile.read_text()
                    self.assertTrue(
                        content.startswith("# keep without final newline\n")
                    )
                    self.assertEqual(content.count("export PATH="), 1)
                    result = subprocess.check_output(
                        [
                            executable,
                            "-f",
                            "-c",
                            '. "$1"; . "$1"; shine; printf "\\n%s" "$PATH"',
                            "test",
                            str(profile),
                        ],
                        env=self.env,
                        text=True,
                    )
                    self.assertEqual(
                        result,
                        "installed-binary\n" + str(directory) + ":" + self.env["PATH"],
                    )
                    immediate = [
                        x[2:] for x in output.splitlines() if x.startswith("  ")
                    ][-1]
                    result = subprocess.check_output(
                        [
                            executable,
                            "-f",
                            "-c",
                            immediate
                            + "; "
                            + immediate
                            + '; shine; printf "\\n%s" "$PATH"',
                        ],
                        env=self.env,
                        text=True,
                    )
                    self.assertEqual(
                        result,
                        "installed-binary\n" + str(directory) + ":" + self.env["PATH"],
                    )

    def test_missing_profile_is_created_only_after_running_hint(self):
        for shell in ("bash", "zsh"):
            with self.subTest(shell=shell):
                executable = shutil.which(shell)
                if not executable:
                    self.skipTest(f"{shell} is not installed")
                directory = self.home / ".local/bin"
                output = self.install("/bin/" + shell, directory)
                profile = self.home / (".bashrc" if shell == "bash" else ".zshrc")
                self.assertFalse(profile.exists())
                command = next(x[2:] for x in output.splitlines() if x.startswith("  "))
                subprocess.run(
                    [executable, "-f", "-c", command], env=self.env, check=True
                )
                result = subprocess.check_output(
                    [executable, "-f", "-c", '. "$1"; shine', "test", str(profile)],
                    env=self.env,
                    text=True,
                )
                self.assertEqual(result, "installed-binary")

    def test_unknown_or_unset_shell_does_not_guess_profile(self):
        for shell in (None, "", "/bin/tcsh"):
            output = self.install(shell, self.home / ".local/bin")
            self.assertIn("only provided for Bash and Zsh", output)
            self.assertNotIn("  ", output)
            self.assertFalse((self.home / ".bashrc").exists())

    def test_bash_startup_reads_hint_in_fresh_and_existing_homes(self):
        profiles = (".bash_profile", ".bash_login", ".profile")
        for existing in ((), (".profile",), (".bash_login", ".profile"), profiles):
            for sources_rc in (False, True):
                with self.subTest(existing=existing, sources_rc=sources_rc):
                    for name in (*profiles, ".bashrc"):
                        (self.home / name).unlink(missing_ok=True)
                    original = "# existing configuration\n"
                    if sources_rc:
                        original += '[ ! -r "$HOME/.bashrc" ] || . "$HOME/.bashrc"\n'
                    for name in existing:
                        (self.home / name).write_text(original)
                    directory = self.home / ".local/bin"
                    output = self.install("/bin/bash", directory)
                    command = next(
                        x[2:] for x in output.splitlines() if x.startswith("  ")
                    )
                    for _ in range(2):
                        subprocess.run(
                            ["bash", "-c", command], env=self.env, check=True
                        )
                    for flags in ("-ic", "-lic"):
                        result = subprocess.run(
                            ["bash", flags, 'shine && printf "\\n%s" "$PATH"'],
                            env=self.env,
                            text=True,
                            capture_output=True,
                        )
                        self.assertEqual(result.returncode, 0, result.stderr)
                        self.assertTrue(
                            result.stdout.startswith("installed-binary\n"), result
                        )
                        self.assertEqual(
                            result.stdout.split("\n")[-1]
                            .split(":")
                            .count(str(directory)),
                            1,
                        )
                    selected = existing[0] if existing else ".bash_profile"
                    for name in profiles:
                        path = self.home / name
                        if name == selected:
                            self.assertTrue(path.exists(), name)
                            self.assertEqual(path.read_text().count("export PATH="), 1)
                            if name in existing:
                                self.assertTrue(path.read_text().startswith(original))
                        elif name in existing:
                            self.assertEqual(path.read_text(), original)
                        else:
                            self.assertFalse(path.exists(), name)

    def test_existing_path_has_no_hint(self):
        directory = self.home / ".local/bin"
        self.env["PATH"] += ":" + str(directory)
        output = self.install("/bin/bash", directory)
        self.assertNotIn("Warning:", output)

    def color_output(self, terminal_stream=None, **overrides):
        env = {**self.env, "TERM": "xterm-256color", "SHELL": "/bin/bash"}
        env.pop("NO_COLOR", None)
        env.update(overrides)
        master, slave = pty.openpty()
        chunks = []
        read_errors = []

        def drain():
            while True:
                try:
                    chunk = os.read(master, 4096)
                except OSError as error:
                    if error.errno != errno.EIO:
                        read_errors.append(error)
                    return
                if not chunk:
                    return
                chunks.append(chunk)

        reader = threading.Thread(target=drain, daemon=True)
        reader.start()
        try:
            with tempfile.TemporaryFile() as stdout, tempfile.TemporaryFile() as stderr:
                try:
                    result = subprocess.run(
                        ["/bin/sh"],
                        input=INSTALLER.read_bytes(),
                        env=env,
                        stdout=slave if terminal_stream == "stdout" else stdout,
                        stderr=slave if terminal_stream == "stderr" else stderr,
                        timeout=10,
                    )
                finally:
                    os.close(slave)
                    reader.join(timeout=5)
                self.assertFalse(reader.is_alive(), "PTY reader did not finish")
                self.assertEqual(read_errors, [])
                terminal = b"".join(chunks)
                stdout.seek(0)
                stderr.seek(0)
                return result.returncode, tuple(
                    (terminal if terminal_stream == stream else file.read())
                    .decode()
                    .replace("\r\n", "\n")
                    for stream, file in (("stdout", stdout), ("stderr", stderr))
                )
        finally:
            os.close(master)

    def test_colors_follow_stdout_without_coloring_copy_commands(self):
        code, (plain, errors) = self.color_output()
        self.assertEqual(code, 0)
        self.assertEqual(errors, "")
        self.assertNotIn("\x1b", plain)
        code, (colored, errors) = self.color_output("stdout")
        self.assertEqual(code, 0)
        self.assertEqual(errors, "")
        for prefix in ("\x1b[32mInstalled", "\x1b[33mWarning:", "\x1b[1mFor "):
            self.assertIn(prefix, colored)
        self.assertEqual(re.sub(r"\x1b\[[0-9;]*m", "", colored), plain)
        self.assertEqual(
            [line for line in colored.splitlines() if line.startswith("  ")],
            [line for line in plain.splitlines() if line.startswith("  ")],
        )
        for overrides in (
            {"NO_COLOR": "1"},
            {"NO_COLOR": ""},
            {"TERM": "dumb"},
            {"TERM": ""},
        ):
            with self.subTest(overrides=overrides):
                self.assertEqual(
                    self.color_output("stdout", **overrides), (0, (plain, ""))
                )
        self.assertEqual(self.color_output("stderr"), (0, (plain, "")))

    def test_error_color_follows_stderr_and_preserves_exit_status(self):
        uname = self.bin / "uname"
        uname.write_text("#!/bin/sh\nprintf 'UnsupportedOS\\n'\n")
        uname.chmod(0o755)
        plain = "error: unsupported operating system: UnsupportedOS\n"
        self.assertEqual(self.color_output(), (1, ("", plain)))
        self.assertEqual(self.color_output("stdout"), (1, ("", plain)))
        self.assertEqual(
            self.color_output("stderr"),
            (1, ("", "\x1b[31m" + plain.rstrip("\n") + "\x1b[0m\n")),
        )
        for overrides in ({"NO_COLOR": "1"}, {"NO_COLOR": ""}, {"TERM": "dumb"}):
            with self.subTest(overrides=overrides):
                self.assertEqual(
                    self.color_output("stderr", **overrides), (1, ("", plain))
                )

    def test_pasted_hint_preserves_bang_paths_with_history_expansion(self):
        directory = self.home / "quote' bang!shine_missing_event/bin"
        output = self.install("/bin/bash", directory)
        commands = [x[2:] for x in output.splitlines() if x.startswith("  ")]
        # Read commands from stdin: bash -c does not exercise history expansion.
        result = subprocess.run(
            ["bash", "--noprofile", "--norc", "--noediting", "-i"],
            input="set -H\n"
            + "\n".join(commands + commands)
            + '\nshine && printf "\\n%s\\n" "$PATH"\nexit\n',
            env={**self.env, "HISTFILE": "/dev/null", "TERM": "dumb"},
            text=True,
            capture_output=True,
            timeout=10,
        )
        self.assertNotIn("event not found", result.stderr)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            result.stdout,
            "installed-binary\n" + str(directory) + ":" + self.env["PATH"] + "\n",
        )
        for name in (".bashrc", ".bash_profile"):
            self.assertEqual((self.home / name).read_text().count("export PATH="), 1)
        for flags in ("-ic", "-lic"):
            result = subprocess.run(
                ["bash", flags, "shine"],
                env=self.env,
                text=True,
                capture_output=True,
                timeout=10,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout, "installed-binary")

    def test_bash_rejects_unusable_profiles_without_partial_writes(self):
        for name in (".bashrc", ".bash_profile"):
            for kind in ("directory", "dangling_symlink", "read_only", "unreadable"):
                with self.subTest(name=name, kind=kind):
                    for profile in (".bashrc", ".bash_profile", ".profile"):
                        path = self.home / profile
                        if path.is_dir():
                            path.rmdir()
                        elif path.exists() or path.is_symlink():
                            path.unlink()
                    fallback = self.home / ".profile"
                    fallback.write_text("# existing fallback\n")
                    blocked = self.home / name
                    if kind == "directory":
                        blocked.mkdir()
                    elif kind == "dangling_symlink":
                        blocked.symlink_to(self.home / "missing")
                    else:
                        blocked.write_text("# preserve\n")
                        blocked.chmod(0o400 if kind == "read_only" else 0o200)
                        if os.geteuid() == 0:
                            continue  # root bypasses mode bits
                    output = self.install("/bin/bash", self.home / ".local/bin")
                    command = next(
                        x[2:] for x in output.splitlines() if x.startswith("  ")
                    )
                    result = subprocess.run(
                        ["bash", "-c", command],
                        env=self.env,
                        text=True,
                        capture_output=True,
                    )
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn("Cannot configure PATH", result.stderr)
                    self.assertEqual(fallback.read_text(), "# existing fallback\n")
                    other = self.home / (
                        ".bash_profile" if name == ".bashrc" else ".bashrc"
                    )
                    self.assertFalse(other.exists())
                    if kind in ("read_only", "unreadable"):
                        blocked.chmod(0o600)
                        self.assertEqual(blocked.read_text(), "# preserve\n")

    def test_fish_uses_manual_hint_without_writing_profile(self):
        directory = self.home / "fish bin"
        output = self.install("/usr/bin/fish", directory)
        self.assertIn("Add the installation directory to PATH", output)
        self.assertNotIn("fish_add_path", output)
        self.assertFalse(any(line.startswith("  ") for line in output.splitlines()))
        self.assertEqual(list(self.home.iterdir()), [directory])


if __name__ == "__main__":
    unittest.main()
