"""Reject Linux release binaries requiring glibc newer than Ubuntu 22.04."""

import os
from pathlib import Path
import re
import subprocess
import sys

MAX_GLIBC = (2, 35)


def check_versions(output):
    # Use version requirements, including weak references; not arbitrary strings
    # embedded in presets or the version-symbol/definition tables.
    _, found, needs = output.partition("Version needs section")
    names = set(re.findall(r"\bName: (GLIBC_\S+)", needs)) if found else set()
    if not names:
        raise ValueError("no glibc version requirements found")
    versions = []
    for name in sorted(names):
        match = re.fullmatch(r"GLIBC_(\d+)\.(\d+)(?:\.(\d+))?", name)
        if not match:
            raise ValueError(f"unsupported glibc requirement: {name}")
        version = tuple(int(part or 0) for part in match.groups())
        if version > (*MAX_GLIBC, 0):
            raise ValueError(f"{name} exceeds the GLIBC_2.35 release baseline")
        versions.append(version)
    return max(versions)


def main():
    if len(sys.argv) != 2:
        sys.exit("usage: check-linux-glibc.py BINARY")
    binary = Path(sys.argv[1])
    try:
        result = subprocess.run(
            ["readelf", "--version-info", "--wide", str(binary)],
            env={**os.environ, "LC_ALL": "C"},
            check=True,
            capture_output=True,
            text=True,
        )
        version = check_versions(result.stdout)
    except (OSError, subprocess.CalledProcessError, ValueError) as error:
        sys.exit(f"{binary}: glibc compatibility check failed: {error}")
    print(f"{binary}: maximum GLIBC_{version[0]}.{version[1]} <= GLIBC_2.35")


if __name__ == "__main__":
    main()
