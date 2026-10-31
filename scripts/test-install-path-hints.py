"""Run with python3 scripts/test-install-path-hints.py; no downloads or real profiles."""

import io
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
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

    def test_fish_uses_manual_hint_without_writing_profile(self):
        directory = self.home / "fish bin"
        output = self.install("/usr/bin/fish", directory)
        self.assertIn("Add the installation directory to PATH", output)
        self.assertNotIn("fish_add_path", output)
        self.assertFalse(any(line.startswith("  ") for line in output.splitlines()))
        self.assertEqual(list(self.home.iterdir()), [directory])


if __name__ == "__main__":
    unittest.main()
