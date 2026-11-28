"""Offline end-to-end installer checks; run with python3 on Unix or Windows."""

import errno
import io
from itertools import product
import os
import shutil
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import threading
import unittest

ROOT = Path(__file__).resolve().parents[1]
WINDOWS = os.name == "nt"


def prepare_preview_installer(output):
    subprocess.run(
        [
            sys.executable,
            str(ROOT / "scripts/prepare-preview-installer.py"),
            str(output),
        ],
        check=True,
    )


class ReleaseInstallers(unittest.TestCase):
    def run_installer(
        self,
        version=None,
        target=None,
        failure="",
        custom=False,
        existing=True,
        preview_asset=False,
        downloader="curl",
        terminal=False,
        term="xterm",
        no_color=False,
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
                TEST_PROGRESS="yes"
                if terminal == "stderr" and term != "dumb"
                else "no",
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
            env.pop("TERM", None)
            env.pop("NO_COLOR", None)
            if term is not None:
                env["TERM"] = term
            if no_color:
                env["NO_COLOR"] = "1"
            if preview_asset:
                installer = root / ("install.ps1" if WINDOWS else "install.sh")
                prepare_preview_installer(installer)
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
if [ "$1" = -fSL ]; then
  [ "$TEST_PROGRESS" = yes ] && [ "$2" = --progress-bar ] || exit 90
  shift
else
  [ "$1" = -fsSL ] || exit 91
  case "$2" in */releases/latest) ;; *) [ "$TEST_PROGRESS" = no ] || exit 92 ;; esac
fi
printf '%s\n' "$2" >> "$TEST_REQUESTS"
case "$2" in
  */releases/latest) [ "$1" = -fsSL ] && [ "$#" = 2 ] || exit 94
     printf '{"tag_name": "v2.8.0"}\n' ;;
  *) [ "$#" = 4 ] && [ "$3" = -o ] || exit 93
     if [ "$TEST_FAILURE" = download ]; then
       printf partial > "$4"
       printf 'simulated download failure\n' >&2
       exit 22
     fi
     cp "$TEST_ARCHIVE" "$4" ;;
esac
""",
                    "wget": """#!/bin/sh
case "$1" in
  -qO-) printf '%s\n' "$2" >> "$TEST_REQUESTS"
        printf '{"tag_name": "v2.8.0"}\n'; exit ;;
  -O) [ "$TEST_PROGRESS" = yes ] || exit 90 ;;
  -qO) [ "$TEST_PROGRESS" = no ] || exit 91 ;;
  *) exit 92 ;;
esac
[ "$#" = 3 ] || exit 93
printf '%s\n' "$3" >> "$TEST_REQUESTS"
if [ "$TEST_FAILURE" = download ]; then
  printf partial > "$2"
  printf 'simulated download failure\n' >&2
  exit 22
fi
cp "$TEST_ARCHIVE" "$2"
""",
                    "uname": '#!/bin/sh\ncase "$1" in -s) echo '
                    + target[0]
                    + ";; -m) echo "
                    + target[1]
                    + ";; esac\n",
                }
                if downloader == "wget":
                    del scripts["curl"]
                for name, content in scripts.items():
                    path = mocks / name
                    path.write_text(content)
                    path.chmod(0o755)
                # Isolate discovery so wget cases cannot find the host's curl.
                for name in (
                    "cp",
                    "tar",
                    "gzip",
                    "mktemp",
                    "rm",
                    "mkdir",
                    "mv",
                    "chmod",
                    "grep",
                    "sed",
                ):
                    (mocks / name).symlink_to(shutil.which(name))
                env["PATH"] = str(mocks)
                argv = ["/bin/sh"]
                asset_target = (
                    ("darwin" if target[0] == "Darwin" else "linux")
                    + "-"
                    + ("aarch64" if target[1] in ("arm64", "aarch64") else "x86_64")
                )
            installer_input = None
            if not WINDOWS:
                installer_input = Path(env["TEST_INSTALLER"]).read_text()
                if term is None:
                    # Some /bin/sh implementations initialize absent TERM to dumb.
                    installer_input = "unset TERM\n" + installer_input
            master = slave = None
            chunks = []
            read_errors = []
            if terminal:
                import pty

                master, slave = pty.openpty()

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
                try:
                    result = subprocess.run(
                        argv,
                        env=env,
                        input=installer_input,
                        stdout=slave if terminal == "stdout" else subprocess.PIPE,
                        stderr=slave if terminal == "stderr" else subprocess.PIPE,
                        text=True,
                        timeout=30,
                    )
                finally:
                    if terminal:
                        os.close(slave)
                        reader.join(timeout=5)
                if terminal:
                    self.assertFalse(reader.is_alive(), "PTY reader did not finish")
                    self.assertEqual(read_errors, [])
                    setattr(result, terminal, b"".join(chunks).decode())
            finally:
                if terminal:
                    os.close(master)
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
                self.assertNotIn("Installed shine", result.stdout)
                if not WINDOWS and failure == "download":
                    self.assertEqual(result.returncode, 22, result.stderr)
                    self.assertIn("simulated download failure", result.stderr)
            else:
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertEqual(binary.read_bytes(), b"replacement")

    @unittest.skipIf(WINDOWS, "POSIX downloader and terminal behavior")
    def test_download_progress(self):
        for downloader in ("curl", "wget"):
            for terminal, term in (
                (False, "xterm"),
                ("stderr", "xterm"),
                ("stderr", "dumb"),
                ("stderr", ""),
                ("stderr", None),
                ("stdout", "xterm"),
            ):
                for failure in ("", "download"):
                    with self.subTest(
                        downloader=downloader,
                        terminal=terminal,
                        term=term,
                        failure=failure,
                    ):
                        self.run_installer(
                            downloader=downloader,
                            terminal=terminal,
                            term=term,
                            failure=failure,
                        )

    @unittest.skipIf(WINDOWS, "POSIX downloader and terminal behavior")
    def test_progress_with_no_color_and_custom_destination(self):
        for downloader in ("curl", "wget"):
            with self.subTest(downloader=downloader):
                self.run_installer(
                    downloader=downloader,
                    terminal="stderr",
                    no_color=True,
                    custom=True,
                    preview_asset=True,
                )

    def test_versions_and_overrides(self):
        for version, custom in product(
            (None, "latest", "2.7.0", "preview"), (False, True)
        ):
            with self.subTest(installer="stable", version=version, custom=custom):
                self.run_installer(version, custom=custom)
            with self.subTest(installer="preview", version=version, custom=custom):
                self.run_installer(
                    version, preview_asset=True, existing=False, custom=custom
                )

    def test_preview_asset_generation(self):
        for name, default, preview in (
            ("install.sh", "${SHINE_VERSION:-latest}", "${SHINE_VERSION:-preview}"),
            ("install.ps1", "else { 'latest' }", "else { 'preview' }"),
        ):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as temp:
                source = (ROOT / name).read_text(encoding="utf-8")
                output = Path(temp) / name
                prepare_preview_installer(output)
                generated = output.read_text(encoding="utf-8")
                self.assertNotEqual(generated, source)
                self.assertEqual(generated.replace(preview, default), source)
                self.assertEqual((ROOT / name).read_text(encoding="utf-8"), source)

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
            with self.subTest(installer="stable", failure=failure):
                self.run_installer("preview", failure=failure)
            with self.subTest(installer="preview", failure=failure):
                self.run_installer(preview_asset=True, failure=failure)


if __name__ == "__main__":
    unittest.main()
