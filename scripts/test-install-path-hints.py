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
            for name in ("simple bin", "quote' dollar$ back` slash\\ double\" bin"):
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
                            '. "$1"; shine; printf "\\n%s" "$PATH"',
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

    def test_fish_uses_manual_hint_without_writing_profile(self):
        directory = self.home / "fish bin"
        output = self.install("/usr/bin/fish", directory)
        self.assertIn("Add the installation directory to PATH", output)
        self.assertNotIn("fish_add_path", output)
        self.assertFalse(any(line.startswith("  ") for line in output.splitlines()))
        self.assertEqual(list(self.home.iterdir()), [directory])


if __name__ == "__main__":
    unittest.main()
