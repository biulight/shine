"""Check --version output against the source commit and commit date before upload."""

import datetime
import re
import sys


def check_version(output, sha, date):
    if not re.fullmatch(r"[0-9a-f]{40}", sha):
        raise ValueError("expected the full Git commit SHA")
    if not re.fullmatch(r"\d{4}-\d{2}-\d{2}", date):
        raise ValueError("expected an ISO commit date")
    datetime.date.fromisoformat(date)
    provenance = f"({sha[:9]} {date})"
    semver = r"[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?"
    if not re.fullmatch(
        r"shine " + semver + " " + re.escape(provenance), output.strip()
    ):
        raise ValueError(
            f"expected shine <version> {provenance}; got {output.strip()!r}"
        )


def main():
    if len(sys.argv) != 3:
        sys.exit(
            "usage: shine --version | check-release-version.py FULL_SHA COMMIT_DATE"
        )
    output = sys.stdin.read()
    try:
        check_version(output, sys.argv[1], sys.argv[2])
    except ValueError as error:
        sys.exit(f"release version check failed: {error}")
    print(output.strip())


if __name__ == "__main__":
    main()
