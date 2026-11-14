"""Offline regressions for the release version output gate."""

import importlib.util
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).with_name("check-release-version.py")
spec = importlib.util.spec_from_file_location("check_release_version", SCRIPT)
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)
SHA = "123456789" + "a" * 31
DATE = "2026-11-14"


class ReleaseVersion(unittest.TestCase):
    def test_complete_stable_preview_and_prerelease_versions(self):
        for version in (
            "2.10.0",
            "2.10.0-preview",
            "2.11.0-rc.1.preview",
            "2.10.0+dev.1",
        ):
            with self.subTest(version=version):
                checker.check_version(
                    f"shine {version} ({SHA[:9]} {DATE})\r\n", SHA, DATE
                )

    def test_rejects_missing_or_partial_provenance(self):
        for suffix in ("", f" ({SHA[:9]})", f" ({DATE})"):
            with self.subTest(suffix=suffix), self.assertRaises(ValueError):
                checker.check_version("shine 2.10.0-preview" + suffix, SHA, DATE)

    def test_rejects_wrong_commit_or_date(self):
        for sha, date in (("987654321", DATE), (SHA[:9], "2026-11-13"), (SHA, DATE)):
            with self.subTest(sha=sha, date=date), self.assertRaises(ValueError):
                checker.check_version(f"shine 2.10.0-preview ({sha} {date})", SHA, DATE)

    def test_rejects_empty_or_extra_output(self):
        good = f"shine 2.10.0-preview ({SHA[:9]} {DATE})"
        for output in (
            "",
            "warning\n" + good,
            good + "\nextra",
            good.replace("2.10.0-preview", "unknown"),
        ):
            with self.subTest(output=output), self.assertRaises(ValueError):
                checker.check_version(output, SHA, DATE)

    def test_rejects_invalid_expected_metadata(self):
        for sha, date in (("", DATE), (SHA[:8], DATE), (SHA, ""), (SHA, "2026-02-30")):
            with self.subTest(sha=sha, date=date), self.assertRaises(ValueError):
                checker.check_version("shine 2.10.0-preview", sha, date)

    def test_cli_exit_status_and_output(self):
        good = f"shine 2.10.0-preview ({SHA[:9]} {DATE})\n"
        for output, code in ((good, 0), ("shine 2.10.0-preview\n", 1)):
            with self.subTest(output=output):
                result = subprocess.run(
                    [sys.executable, str(SCRIPT), SHA, DATE],
                    input=output,
                    text=True,
                    capture_output=True,
                )
                self.assertEqual(result.returncode, code, result.stderr)
                if code == 0:
                    self.assertEqual(result.stdout, good)
                else:
                    self.assertIn("release version check failed", result.stderr)

    @unittest.skipIf(os.name == "nt", "Linux container setup uses a POSIX shell")
    def test_container_checkout_trust_is_scoped_and_survives_checkout(self):
        # Execute the production workflow command, not a copied trust setup.
        workflow = SCRIPT.parents[1] / ".github/workflows/package-assets.yml"
        lines = workflow.read_text().splitlines()
        start = lines.index("      - name: Trust checkout in Linux container") + 1
        step = []
        for line in lines[start:]:
            if line.startswith("      - "):
                break
            step.append(line)
        command = next(
            line[len("        run: ") :]
            for line in step
            if line.startswith("        run: ")
        )
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory).resolve()
            repo, other = base / "checkout", base / "other"
            env = {
                key: value
                for key, value in os.environ.items()
                if not key.startswith("GIT_")
            }
            env.update(
                HOME=str(base),
                GIT_CONFIG_GLOBAL=str(base / "job.gitconfig"),
                GIT_CONFIG_NOSYSTEM="1",
                LC_ALL="C",
            )

            def git(*args, cwd=repo, check=True):
                return subprocess.run(
                    ["git", *args],
                    cwd=cwd,
                    env=env,
                    text=True,
                    capture_output=True,
                    check=check,
                )

            for path in (repo, other):
                git("init", str(path), cwd=base)
            git(
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "--allow-empty",
                "-m",
                "fixture",
            )
            expected = git("rev-parse", "HEAD").stdout
            # Checkout wrote trust into a separate temporary global config.
            git(
                "config",
                "--file",
                str(base / "checkout.gitconfig"),
                "--add",
                "safe.directory",
                str(repo),
            )
            env.update(GIT_TEST_ASSUME_DIFFERENT_OWNER="1", GITHUB_WORKSPACE=str(repo))
            before = git("rev-parse", "HEAD", check=False)
            self.assertEqual(before.returncode, 128, before.stderr)
            self.assertIn("dubious ownership", before.stderr)
            subprocess.run(["sh", "-e", "-c", command], cwd=repo, env=env, check=True)
            self.assertEqual(git("rev-parse", "HEAD").stdout, expected)
            self.assertRegex(
                git("show", "-s", "--format=%cs", "HEAD").stdout.strip(),
                r"^\d{4}-\d{2}-\d{2}$",
            )
            self.assertEqual(
                git(
                    "config", "--global", "--get-all", "safe.directory"
                ).stdout.splitlines(),
                [str(repo)],
            )
            rejected = git("rev-parse", "--git-dir", cwd=other, check=False)
            self.assertEqual(rejected.returncode, 128, rejected.stderr)
            self.assertIn("dubious ownership", rejected.stderr)


if __name__ == "__main__":
    unittest.main()
