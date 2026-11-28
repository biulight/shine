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
        curl.write_text(
            '#!/bin/sh\n[ "$1" != -fSL ] || shift\ncp "$TEST_ARCHIVE" "$4"\n'
        )
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

    def configuration_command(self, output):
        if "configure future shells:\n" not in output:
            return ""
        return (
            output.split("configure future shells:\n", 1)[1]
            .split("Then run this", 1)[0]
            .strip()
        )

    def apply_bash_hint(self, command):
        subprocess.run(["bash", "-c", command], env=self.env, check=True)

    def assert_no_bash_hint(self, directory):
        output = self.install("/bin/bash", directory)
        self.assertNotIn("configure future shells", output)

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
                    profile_dir = self.home if shell == "bash" else self.root / "z dir"
                    profile_dir.mkdir(exist_ok=True)
                    profile = profile_dir / (".bashrc" if shell == "bash" else ".zshrc")
                    profile.write_text("# keep without final newline")
                    if shell == "zsh":
                        self.env["ZDOTDIR"] = str(profile_dir)
                    output = self.install("/bin/" + shell, directory)
                    command = self.configuration_command(output)
                    for _ in range(2):
                        subprocess.run(
                            [executable, "-f", "-c", command],
                            env=self.env,
                            check=True,
                        )
                        repeated = self.install("/bin/" + shell, directory)
                        self.assertNotIn("configure future shells", repeated)
                        command = self.configuration_command(repeated)
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
                        "installed-binary\n"
                        + (str(directory) + ":") * 2
                        + self.env["PATH"],
                    )
                    immediate = [
                        x[2:] for x in output.splitlines() if x.startswith("  ")
                    ][-1]
                    result = subprocess.check_output(
                        [
                            executable,
                            "-f",
                            "-c",
                            immediate + '; shine; printf "\\n%s" "$PATH"',
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
                command = self.configuration_command(output)
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
                    command = self.configuration_command(output)
                    self.apply_bash_hint(command)
                    self.assert_no_bash_hint(directory)
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
                            2 if not existing and flags == "-lic" else 1,
                        )
                    selected = existing[0] if existing else ".bash_profile"
                    for name in profiles:
                        path = self.home / name
                        if name == selected:
                            self.assertTrue(path.exists(), name)
                            if sources_rc and existing:
                                self.assertEqual(path.read_text(), original)
                            else:
                                self.assertEqual(
                                    path.read_text().count("export PATH="), 1
                                )
                            if name in existing:
                                self.assertTrue(path.read_text().startswith(original))
                        elif name in existing:
                            self.assertEqual(path.read_text(), original)
                        else:
                            self.assertFalse(path.exists(), name)

    def test_new_bash_login_file_loads_later_integrations_once(self):
        output = self.install("/bin/bash", self.home / ".local/bin")
        login = self.home / ".bash_profile"
        self.assertFalse(login.exists())  # Installation only prints the command.
        command = self.configuration_command(output)
        self.apply_bash_hint(command)
        self.assert_no_bash_hint(self.home / ".local/bin")
        with (self.home / ".bashrc").open("a") as stream:
            stream.write(
                "\nrc_count=$(( ${rc_count:-0} + 1 ))\nalias cat='bat'\nalias ls='eza --icons'\n"
            )
        for flags in ("-ic", "-lic"):
            result = subprocess.run(
                ["bash", flags, 'alias cat; alias ls; test "$rc_count" = 1'],
                env={**self.env, "HISTFILE": "/dev/null"},
                text=True,
                capture_output=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("cat='bat'", result.stdout)
            self.assertIn("ls='eza --icons'", result.stdout)
        result = subprocess.run(
            ["bash", "-lc", 'test -z "${rc_count:-}"'],
            env=self.env,
            text=True,
            capture_output=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "")

    def test_existing_empty_or_path_only_login_file_keeps_startup_logic(self):
        directory = self.home / ".local/bin"
        output = self.install("/bin/bash", directory)
        self.apply_bash_hint(self.configuration_command(output))
        path_line = (self.home / ".bashrc").read_text().strip()
        login = self.home / ".bash_profile"
        for original in ("", path_line + "\n"):
            with self.subTest(original=original):
                login.write_text(original)
                command = self.configuration_command(
                    self.install("/bin/bash", directory)
                )
                self.apply_bash_hint(command)
                self.assertNotIn('. "$HOME/.bashrc"', login.read_text())
                self.assertEqual(login.read_text().count("export PATH="), 1)
        self.env["PATH"] += ":" + str(directory)
        self.assert_no_bash_hint(directory)

    def test_bash_hint_preserves_ubuntu_existing_rc_chain_without_double_loading(self):
        directory = self.home / ".local/bin"
        profile = self.home / ".profile"
        profile.write_text(
            '# existing Ubuntu login setup\nif [ -n "${BASH_VERSION:-}" ]; then\n  . "$HOME/.bashrc"\nfi\n'
        )
        (self.home / ".bashrc").write_text(
            "rc_count=$(( ${rc_count:-0} + 1 ))\nalias cat='bat'\n"
        )
        original = profile.read_text()
        output = self.install("/bin/bash", directory)
        self.assertNotIn(str(profile), self.configuration_command(output))
        self.apply_bash_hint(self.configuration_command(output))
        self.assert_no_bash_hint(directory)
        self.assertEqual(profile.read_text(), original)
        self.assertEqual(profile.read_text().count('. "$HOME/.bashrc"'), 1)
        self.assertFalse((self.home / ".bash_profile").exists())
        result = subprocess.run(
            ["bash", "-lic", 'alias cat; test "$rc_count" = 1'],
            env=self.env,
            text=True,
            capture_output=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("cat='bat'", result.stdout)

    def test_existing_path_has_no_hint(self):
        directory = self.home / ".local/bin"
        self.env["PATH"] += ":" + str(directory)
        output = self.install("/bin/bash", directory)
        self.assertNotIn("Warning:", output)

    def test_bash_common_loader_forms_preserve_read_only_login_profile(self):
        directory = self.home / ".local/bin"
        profile = self.home / ".profile"
        rc = self.home / ".bashrc"
        loaders = (
            '. "$HOME/.bashrc"',
            'source "${HOME}/.bashrc"',
            ". $HOME/.bashrc",
            "source ${HOME}/.bashrc",
            ". ~/.bashrc; # interactive configuration",
            '[ -f "$HOME/.bashrc" ] && . "$HOME/.bashrc"',
            '[ ! -r "$HOME/.bashrc" ] || source "$HOME/.bashrc"',
            "if [ -f ~/.bashrc ]; then source ~/.bashrc; fi",
            'if [ -n "$BASH_VERSION" ]; then\n  . "$HOME/.bashrc"\nfi',
        )
        for loader in loaders:
            with self.subTest(loader=loader):
                # Normal .bashrc files may return early in non-interactive shells.
                rc.write_text("case $- in *i*) ;; *) return ;; esac\n")
                profile.write_text(loader + "\n")
                profile.chmod(0o444)
                try:
                    output = self.install("/bin/bash", directory)
                    command = self.configuration_command(output)
                    self.assertIn(str(rc), command)
                    self.assertNotIn(str(profile), command)
                    self.assertNotIn("Cannot", output)
                    self.apply_bash_hint(command)
                    self.assertEqual(profile.read_text(), loader + "\n")
                    for flags in ("-ic", "-lic"):
                        result = subprocess.run(
                            ["bash", flags, "shine"],
                            env=self.env,
                            capture_output=True,
                            text=True,
                        )
                        self.assertEqual(result.returncode, 0, result.stderr)
                        self.assertEqual(result.stdout, "installed-binary")
                    self.assert_no_bash_hint(directory)
                finally:
                    profile.chmod(0o644)

    def test_bash_loader_fallback_uses_only_selected_login_profile(self):
        directory = self.home / ".local/bin"
        profile = self.home / ".bash_profile"
        lower_profile = self.home / ".profile"
        lower_profile.write_text('. "$HOME/.bashrc"\n')
        for original in (
            '# . "$HOME/.bashrc"\n',
            'printf "%s\\n" \'source "$HOME/.bashrc"\'\n',
            '. "$HOME/.bashrc.other"\n',
            '[ -r "$HOME/.bashrc" ] || . "$HOME/.bashrc"\n',
        ):
            with self.subTest(original=original):
                (self.home / ".bashrc").write_text("# existing rc\n")
                profile.write_text(original)
                output = self.install("/bin/bash", directory)
                command = self.configuration_command(output)
                self.assertIn(str(profile), command)
                self.assertNotIn(str(lower_profile), command)
                self.apply_bash_hint(command)
                self.assertTrue(profile.read_text().startswith(original))
                self.assertEqual(profile.read_text().count("export PATH="), 1)
                self.assertEqual(lower_profile.read_text(), '. "$HOME/.bashrc"\n')

    def test_bash_loader_hash_suffix_is_not_a_comment(self):
        directory = self.home / ".local/bin"
        profile = self.home / ".profile"
        rc = self.home / ".bashrc"
        (self.home / ".bashrc#login").write_text("loader_seen=alternate\n")
        cases = (
            ('. "$HOME/.bashrc"#login', False),
            (". ~/.bashrc#login", False),
            ('[ -f "$HOME/.bashrc" ] && . "$HOME/.bashrc"#login', False),
            ('. "$HOME/.bashrc" # comment', True),
            ('. "$HOME/.bashrc"\t# comment', True),
            ('. "$HOME/.bashrc";# comment', True),
            ("if [ -f ~/.bashrc ]; then . ~/.bashrc; fi # comment", True),
            ("if [ -f ~/.bashrc ]; then . ~/.bashrc; fi;# comment", True),
        )
        for loader, loads_rc in cases:
            with self.subTest(loader=loader):
                rc.write_text("loader_seen=rc\n")
                profile.write_text(loader + "\n")
                output = self.install("/bin/bash", directory)
                self.apply_bash_hint(self.configuration_command(output))
                if loads_rc:
                    self.assertEqual(profile.read_text(), loader + "\n")
                else:
                    self.assertTrue(profile.read_text().startswith(loader + "\n"))
                    self.assertEqual(profile.read_text().count("export PATH="), 1)
                result = subprocess.run(
                    ["bash", "-lic", 'shine && printf "\\n%s" "$loader_seen"'],
                    env=self.env,
                    capture_output=True,
                    text=True,
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(
                    result.stdout,
                    "installed-binary\n" + ("rc" if loads_rc else "alternate"),
                )
                self.assert_no_bash_hint(directory)

    def test_bash_existing_profiles_receive_one_simple_command(self):
        for name in (".bashrc", ".profile"):
            (self.home / name).write_text("# keep without final newline")
        directory = self.home / ".local/bin"
        output = self.install("/bin/bash", directory)
        command = self.configuration_command(output)
        self.assertEqual(len(command.splitlines()), 1)
        self.assertIn("| tee -a -- ", command)
        self.assertNotIn('case ":$PATH:"', command)
        for obsolete in ("grep", "shine_new_login", "IFS=", "for shine_file"):
            self.assertNotIn(obsolete, command)
        result = subprocess.run(
            ["bash", "-c", command], env=self.env, capture_output=True, check=True
        )
        self.assertEqual(result.stdout, b"")
        for name in (".bashrc", ".profile"):
            content = (self.home / name).read_text()
            self.assertTrue(content.startswith("# keep without final newline\n"))
            self.assertEqual(content.count("export PATH="), 1)
        repeated = self.install("/bin/bash", directory)
        self.assertNotIn("configure future shells", repeated)
        immediate = repeated.split("Then run this", 1)[1].split(":\n", 1)[1].strip()
        self.assertTrue(immediate.startswith("export PATH="))

    def test_bash_legacy_and_partial_configuration_only_print_missing_writes(self):
        directory = self.home / ".local/bin"
        # Literal fixtures from both previous installer renderings.
        for quoted_directory in (str(directory), f"'{directory}'"):
            legacy = (
                f'case ":$PATH:" in *:{quoted_directory}:*) ;; *) '
                f'export PATH={quoted_directory}"${{PATH:+:$PATH}}" ;; esac\n'
            )
            for configured in (".bashrc", ".profile"):
                with self.subTest(
                    configured=configured, quoted_directory=quoted_directory
                ):
                    missing = ".profile" if configured == ".bashrc" else ".bashrc"
                    (self.home / configured).write_text(legacy)
                    (self.home / missing).write_text("# preserve\n")
                    output = self.install("/bin/bash", directory)
                    command = self.configuration_command(output)
                    self.assertNotIn(str(self.home / configured), command)
                    self.assertIn(">> " + str(self.home / missing), command)
                    self.apply_bash_hint(command)
                    self.assertEqual((self.home / configured).read_text(), legacy)
                    self.assert_no_bash_hint(directory)

    def test_bash_hint_preserves_shared_profile_symlink_and_permissions(self):
        target = self.home / "shared config"
        target.write_text("# existing configuration without newline")
        target.chmod(0o640)
        for name in (".bashrc", ".profile"):
            (self.home / name).symlink_to(target)
        output = self.install("/bin/bash", self.home / ".local/bin")
        self.assertEqual(target.read_text(), "# existing configuration without newline")
        command = self.configuration_command(output)
        self.apply_bash_hint(command)
        self.assert_no_bash_hint(self.home / ".local/bin")
        self.assertFalse((self.home / ".bash_profile").exists())
        self.assertEqual(target.stat().st_mode & 0o777, 0o640)
        self.assertTrue(
            target.read_text().startswith("# existing configuration without newline\n")
        )
        self.assertEqual(target.read_text().count("export PATH="), 1)
        for name in (".bashrc", ".profile"):
            self.assertTrue((self.home / name).is_symlink())
        result = subprocess.run(
            ["bash", "-lic", "shine"], env=self.env, text=True, capture_output=True
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "installed-binary")

    def test_bash_hint_deduplicates_hard_link_targets(self):
        rc = self.home / ".bashrc"
        rc.write_text("# existing\n")
        os.link(rc, self.home / ".profile")
        output = self.install("/bin/bash", self.home / ".local/bin")
        self.apply_bash_hint(self.configuration_command(output))
        self.assertEqual(rc.read_text().count("export PATH="), 1)
        self.assertTrue(os.path.samefile(rc, self.home / ".profile"))

    def test_bash_partial_tee_failure_can_be_retried_with_fresh_hint(self):
        directory = self.home / ".local/bin"
        rc = self.home / ".bashrc"
        profile = self.home / ".profile"
        rc.write_text("# rc\n")
        profile.write_text("# login\n")
        output = self.install("/bin/bash", directory)
        profile.unlink()
        profile.mkdir()
        result = subprocess.run(
            ["bash", "-c", self.configuration_command(output)],
            env=self.env,
            capture_output=True,
            text=True,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(".profile", result.stderr)
        self.assertEqual(rc.read_text().count("export PATH="), 1)
        profile.rmdir()
        profile.write_text("# login\n")
        retry = self.configuration_command(self.install("/bin/bash", directory))
        self.assertNotIn(str(rc), retry)
        self.apply_bash_hint(retry)
        for path in (rc, profile):
            self.assertEqual(path.read_text().count("export PATH="), 1)

    def test_bash_hint_reports_write_failure_after_preflight(self):
        output = self.install("/bin/bash", self.home / ".local/bin")
        command = self.configuration_command(output)
        (self.home / ".bashrc").mkdir()
        result = subprocess.run(
            ["bash", "-c", command], env=self.env, text=True, capture_output=True
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(".bashrc", result.stderr)
        self.assertFalse((self.home / ".bash_profile").exists())

    def test_zsh_uses_only_install_time_exported_zdotdir(self):
        executable = shutil.which("zsh")
        if not executable:
            self.skipTest("zsh is not installed")
        custom = self.root / "z custom"
        custom.mkdir()
        directory = self.home / ".local/bin"
        self.env.update(SHELL="/bin/zsh", SHINE_INSTALL_DIR=str(directory))
        for mode in ("unset", "empty", "exported", "unexported"):
            with self.subTest(mode=mode):
                for parent in (custom, self.home):
                    (parent / ".zshrc").unlink(missing_ok=True)
                env = dict(self.env)
                if mode in ("empty", "exported"):
                    env["ZDOTDIR"] = str(custom) if mode == "exported" else ""
                setup = 'ZDOTDIR="$1"; ' if mode == "unexported" else ""
                installer = [
                    executable,
                    "-f",
                    "-c",
                    setup + '/bin/sh "$2"',
                    "test",
                    str(custom),
                    str(INSTALLER),
                ]
                output = subprocess.check_output(installer, env=env, text=True)
                profile = (custom if mode == "exported" else self.home) / ".zshrc"
                command = self.configuration_command(output)
                self.assertIn(str(profile), command)
                self.assertNotIn("ZDOTDIR", command)
                self.assertNotIn("grep", command)
                self.assertFalse(profile.exists())
                subprocess.run(
                    [executable, "-f", "-c", setup + command, "test", str(custom)],
                    env=env,
                    check=True,
                )
                self.assertEqual(profile.read_text().count("export PATH="), 1)
                other = (self.home if mode == "exported" else custom) / ".zshrc"
                self.assertFalse(other.exists())
                repeated = subprocess.check_output(installer, env=env, text=True)
                self.assertNotIn("configure future shells", repeated)
                # Empty/unexported ZDOTDIR falls back in the hint; Zsh itself
                # does not necessarily select HOME in those environments.
                if mode in ("unset", "exported"):
                    result = subprocess.check_output(
                        [executable, "-ic", "shine"],
                        env=env,
                        text=True,
                    )
                    self.assertEqual(result, "installed-binary")

    def test_zsh_rejects_unusable_configuration_destinations(self):
        directory = self.home / ".local/bin"
        profile = self.home / ".zshrc"
        for kind in ("directory", "dangling_symlink", "missing_parent"):
            with self.subTest(kind=kind):
                if kind == "directory":
                    profile.mkdir()
                elif kind == "dangling_symlink":
                    profile.symlink_to(self.home / "missing")
                else:
                    self.env["ZDOTDIR"] = str(self.home / "missing")
                output = self.install("/bin/zsh", directory)
                self.assertIn("Cannot", output)
                self.assertNotIn("configure future shells", output)
                self.assertIn("  export PATH=", output)
                if profile.is_dir():
                    profile.rmdir()
                else:
                    profile.unlink(missing_ok=True)

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

    def test_colors_follow_stdout_and_highlight_copy_commands(self):
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
            [line for line in colored.splitlines() if line.startswith("\x1b[36m")],
            [
                "\x1b[36m" + line + "\x1b[0m"
                for line in plain.splitlines()
                if line.startswith("  ")
            ],
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
        self.home = self.root / "home' bang! dollar$ back` glob*[x]"
        self.home.mkdir()
        self.env["HOME"] = str(self.home)
        directory = self.home / "quote' bang!shine_missing_event/bin"
        output = self.install("/bin/bash", directory)
        commands = [
            self.configuration_command(output),
            output.split("Then run this", 1)[1].split(":\n", 1)[1].strip(),
        ]
        self.assertEqual(len(commands[0].splitlines()), 1)
        self.assertNotIn("shine_new_login", commands[0])
        # Read commands from stdin: bash -c does not exercise history expansion.
        result = subprocess.run(
            ["bash", "--noprofile", "--norc", "--noediting", "-i"],
            input="set -euH\n"
            + "\n".join(commands)
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
        self.assert_no_bash_hint(directory)
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
                    self.assertIn("Cannot configure PATH", output)
                    self.assertNotIn("configure future shells", output)
                    commands = [x for x in output.splitlines() if x.startswith("  ")]
                    self.assertEqual(len(commands), 1)  # current terminal only
                    self.assertEqual(fallback.read_text(), "# existing fallback\n")
                    other = self.home / (
                        ".bash_profile" if name == ".bashrc" else ".bashrc"
                    )
                    self.assertFalse(other.exists())
                    if kind in ("read_only", "unreadable"):
                        blocked.chmod(0o600)
                        self.assertEqual(blocked.read_text(), "# preserve\n")

    def test_newline_paths_are_rejected_before_installation(self):
        for variable in ("HOME", "SHINE_INSTALL_DIR", "ZDOTDIR"):
            for separator in ("\n", "\r"):
                with self.subTest(variable=variable, separator=repr(separator)):
                    directory = self.root / "not-installed"
                    env = {
                        **self.env,
                        "SHINE_INSTALL_DIR": str(directory),
                        "SHELL": "/bin/zsh",
                    }
                    env[variable] = str(self.root / ("bad" + separator + "path"))
                    result = subprocess.run(
                        ["/bin/sh", str(INSTALLER)],
                        env=env,
                        text=True,
                        capture_output=True,
                    )
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn("newline", result.stderr)
                    self.assertIn(variable, result.stderr)
                    self.assertNotIn("Downloading", result.stdout)
                    self.assertFalse(directory.exists())
                    self.assertFalse(Path(env[variable]).exists())

    def test_fish_uses_manual_hint_without_writing_profile(self):
        directory = self.home / "fish bin"
        output = self.install("/usr/bin/fish", directory)
        self.assertIn("Add the installation directory to PATH", output)
        self.assertNotIn("fish_add_path", output)
        self.assertFalse(any(line.startswith("  ") for line in output.splitlines()))
        self.assertEqual(list(self.home.iterdir()), [directory])


if __name__ == "__main__":
    unittest.main()
