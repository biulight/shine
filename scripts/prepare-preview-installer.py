"""Build a preview installer asset without changing the stable source installer."""

import argparse
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    defaults = {
        ".sh": 'SHINE_VERSION="${SHINE_VERSION:-latest}"',
        ".ps1": "$ShineVersion = if ($env:SHINE_VERSION) { $env:SHINE_VERSION } else { 'latest' }",
    }
    if args.output.suffix not in defaults:
        parser.error("output must end in .sh or .ps1")
    source = Path(__file__).resolve().parents[1] / ("install" + args.output.suffix)
    content = source.read_text(encoding="utf-8")
    default = defaults[args.output.suffix]
    if content.count(default) != 1:
        raise SystemExit("expected exactly one installer version default")
    args.output.write_text(
        content.replace(default, default.replace("latest", "preview")),
        encoding="utf-8",
    )


if __name__ == "__main__":
    main()
