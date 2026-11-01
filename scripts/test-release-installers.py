"""Offline end-to-end installer checks; run with python3 on Unix or Windows."""

import io
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
WINDOWS = os.name == "nt"


class ReleaseInstallers(unittest.TestCase):
    def run_installer(
        self,
        version=None,
        target=None,
        failure="",
        custom=False,
        existing=True,
        preview_asset=False,
    ):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            home = root / "home"
            home.mkdir()
            destination = (
                root / "custom bin"
                if custom
                else home / ("Programs/shine" if WINDOWS else ".local/bin")
            )
            binary = destination / ("shine.exe" if WINDOWS else "shine")
            if existing:
                destination.mkdir(parents=True)
                binary.write_bytes(b"previous")
            scratch = root / "scratch"
            scratch.mkdir()
            archive = root / "archive.tar.gz"
            with tarfile.open(archive, "w:gz") as tar:
                data = b"replacement"
                entry = tarfile.TarInfo(
                    "other" if failure == "missing" else binary.name
                )
                entry.size = len(data)
                tar.addfile(entry, io.BytesIO(data))
            requests = root / "requests"
            env = dict(os.environ)
            for key in ("SHINE_VERSION", "SHINE_INSTALL_DIR", "SHINE_REPO"):
                env.pop(key, None)
            env.update(
                TMPDIR=str(scratch),
                TMP=str(scratch),
                TEMP=str(scratch),
                HOME=str(home),
                LOCALAPPDATA=str(home),
                TEST_ARCHIVE=str(archive),
                TEST_REQUESTS=str(requests),
                TEST_FAILURE=failure,
                TEST_INSTALLER=str(ROOT / ("install.ps1" if WINDOWS else "install.sh")),
            )
            if preview_asset:
                installer = root / ("install.ps1" if WINDOWS else "install.sh")
                subprocess.run(
                    [
                        sys.executable,
                        str(ROOT / "scripts/prepare-preview-installer.py"),
                        str(installer),
                    ],
                    check=True,
                )
                env["TEST_INSTALLER"] = str(installer)
            if version is not None:
                env["SHINE_VERSION"] = version
            if custom:
                env.update(
                    SHINE_REPO="example/shine", SHINE_INSTALL_DIR=str(destination)
                )
            if WINDOWS:
                target = target or "x86_64"
                env["PROCESSOR_ARCHITECTURE"] = "AMD64"
                env["PROCESSOR_ARCHITEW6432"] = (
                    "ARM64" if target == "aarch64" else "AMD64"
                )
                wrapper = root / "run.ps1"
                wrapper.write_text(
                    """
$ErrorActionPreference = 'Stop'
function Invoke-RestMethod {
    param($Uri, $Headers)
    Add-Content -LiteralPath $env:TEST_REQUESTS -Value $Uri
    return @{ tag_name = 'v2.8.0' }
}
function Invoke-WebRequest {
    param($Uri, $OutFile, $Headers)
    Add-Content -LiteralPath $env:TEST_REQUESTS -Value $Uri
    if ($env:TEST_FAILURE -eq 'download') { throw 'simulated download failure' }
    Copy-Item -LiteralPath $env:TEST_ARCHIVE -Destination $OutFile
}
$previousVersion = $env:SHINE_VERSION
Get-Content -Raw -LiteralPath $env:TEST_INSTALLER | Invoke-Expression
if ($env:SHINE_VERSION -cne $previousVersion) { throw 'installer changed SHINE_VERSION' }
"""
                )
                argv = ["pwsh", "-NoProfile", "-File", str(wrapper)]
                asset_target = "windows-" + target
            else:
                target = target or ("Linux", "x86_64")
                mocks = root / "bin"
                mocks.mkdir()
                scripts = {
                    "curl": """#!/bin/sh
printf '%s\n' "$2" >> "$TEST_REQUESTS"
case "$2" in
  */releases/latest) printf '{"tag_name": "v2.8.0"}\n' ;;
  *) [ "$TEST_FAILURE" != download ] || exit 22
     cp "$TEST_ARCHIVE" "$4" ;;
esac
""",
                    "uname": '#!/bin/sh\ncase "$1" in -s) echo '
                    + target[0]
                    + ";; -m) echo "
                    + target[1]
                    + ";; esac\n",
                }
                for name, content in scripts.items():
                    path = mocks / name
                    path.write_text(content)
                    path.chmod(0o755)
                env["PATH"] = str(mocks) + ":/usr/bin:/bin"
                argv = ["/bin/sh"]
                asset_target = (
                    ("darwin" if target[0] == "Darwin" else "linux")
                    + "-"
                    + ("aarch64" if target[1] in ("arm64", "aarch64") else "x86_64")
                )
            result = subprocess.run(
                argv,
                env=env,
                input=None if WINDOWS else Path(env["TEST_INSTALLER"]).read_text(),
                capture_output=True,
                text=True,
                timeout=30,
            )
            self.assertEqual(list(scratch.iterdir()), [], result.stdout + result.stderr)
            urls = requests.read_text(encoding="utf-8-sig").splitlines()
            repo = "example/shine" if custom else "biulight/shine"
            if version is None and preview_asset:
                version = "preview"
            resolved = "preview" if version == "preview" else "v" + (version or "2.8.0")
            if version == "latest":
                resolved = "v2.8.0"
            expected = f"https://github.com/{repo}/releases/download/{resolved}/shine-{resolved}-{asset_target}.tar.gz"
            self.assertEqual(urls[-1], expected)
            self.assertEqual(len(urls), 2 if version in (None, "latest") else 1)
            if version in (None, "latest"):
                self.assertEqual(
                    urls[0], f"https://api.github.com/repos/{repo}/releases/latest"
                )
            if failure:
                self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertEqual(binary.read_bytes(), b"previous")
            else:
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertEqual(binary.read_bytes(), b"replacement")

    def test_versions_and_overrides(self):
        for version in (None, "latest", "2.7.0", "preview"):
            for custom in (False, True):
                with self.subTest(version=version, custom=custom):
                    self.run_installer(version, custom=custom)

    def test_preview_asset_generation(self):
        for name, default, preview in (
            ("install.sh", "${SHINE_VERSION:-latest}", "${SHINE_VERSION:-preview}"),
            ("install.ps1", "else { 'latest' }", "else { 'preview' }"),
        ):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as temp:
                source = (ROOT / name).read_text(encoding="utf-8")
                output = Path(temp) / name
                subprocess.run(
                    [
                        sys.executable,
                        str(ROOT / "scripts/prepare-preview-installer.py"),
                        str(output),
                    ],
                    check=True,
                )
                generated = output.read_text(encoding="utf-8")
                self.assertNotEqual(generated, source)
                self.assertEqual(generated.replace(preview, default), source)
                self.assertEqual((ROOT / name).read_text(encoding="utf-8"), source)

    def test_preview_asset_defaults_and_overrides(self):
        for version in (None, "latest", "2.7.0", "preview"):
            for custom in (False, True):
                with self.subTest(version=version, custom=custom):
                    self.run_installer(
                        version, preview_asset=True, existing=False, custom=custom
                    )
        for failure in ("download", "missing"):
            with self.subTest(failure=failure):
                self.run_installer(preview_asset=True, failure=failure)

    def test_preview_fresh_install(self):
        self.run_installer("preview", existing=False)

    def test_preview_platforms(self):
        targets = (
            ("x86_64", "aarch64")
            if WINDOWS
            else (
                ("Linux", "x86_64"),
                ("Linux", "aarch64"),
                ("Darwin", "x86_64"),
                ("Darwin", "arm64"),
            )
        )
        for target in targets:
            with self.subTest(target=target):
                self.run_installer(target=target, preview_asset=True)

    def test_preview_failure_preserves_install(self):
        for failure in ("download", "missing"):
            with self.subTest(failure=failure):
                self.run_installer("preview", failure=failure)


if __name__ == "__main__":
    unittest.main()
