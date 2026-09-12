#!/usr/bin/env python3
"""Controlled hybrid CLI regressions; no real keys or non-stdlib Python dependencies.

Build Shine first, then run: python3 scripts/test-hybrid-secrets.py [path/to/shine]
All config, identities, tool stand-ins, caches and source fixtures use a disposable
sandbox. The fake wrappers are intentionally plaintext and must never be used with
real secrets. For real GPG/age interoperability use the ignored Rust test.
"""

import base64
import hashlib
import json
import os
import pathlib
import re
import subprocess
import sys
import tempfile

if os.name != "posix":
    raise SystemExit("This test requires POSIX shell tool stand-ins.")

binary = str(
    pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "target/debug/shine").resolve()
)
with tempfile.TemporaryDirectory(prefix="shine-hybrid-cli-") as tmp:
    root = pathlib.Path(tmp)
    tools = root / "tools"
    tools.mkdir()
    config = root / "config"
    config.mkdir()
    workspace = root / "shine.workspace.toml"
    identity = root / "identity"
    identity.write_text("test identity")
    trace = root / "trace"
    counter = root / "counter"
    script = r"""#!/bin/sh
tool=${0##*/}
if [ "$1" = --version ]; then
 if [ "$tool" = gpg ]; then printf 'gpg (GnuPG) 2.4.7\n'; else printf 'v1.3.2\n'; fi
 exit 0
fi
operation=encrypt
for arg in "$@"; do
 case "$arg" in --list-keys) operation=list;; --decrypt|-d) operation=decrypt;; esac
 last=$arg
done
if [ "$operation" = list ]; then
 printf 'pub:u:2048:1:0000000000000000:0:0:::::e:\nfpr:::::::::AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA:\n'
 exit 0
fi
if [ "$operation" = decrypt ]; then
 printf 'decrypt-%s\n' "$tool" >> "$TRACE"
 if [ "$CASE" = fail-decrypt ]; then exit 24; fi
 /bin/cat "$last"
 exit 0
fi
input=$(/bin/cat)
if [ "$input" = 'shine hybrid recipient preflight v1' ]; then
 printf 'probe-%s\n' "$tool" >> "$TRACE"
 printf probe
 exit 0
fi
printf 'encrypt-%s\n' "$tool" >> "$TRACE"
if [ "$tool" = age ]; then
 case "$CASE" in
 fail-age) exit 23;;
 edit-workspace) printf '\n# concurrent policy edit\n' >> "$WORKSPACE";;
 edit-source) printf '# external source edit\n' > "$SOURCE";;
 remove-source) /bin/rm "$SOURCE";;
 multi)
  if [ -f "$COUNTER" ]; then printf '\n# policy edited between files\n' >> "$WORKSPACE"; else printf 1 > "$COUNTER"; fi;;
 esac
fi
printf '%s' "$input"
"""
    for name in ["gpg", "age"]:
        tool = tools / name
        tool.write_text(script)
        tool.chmod(0o700)
    (config / "config.toml").write_text(
        'schema_version = 2\nlast_cleared_schema_version = 2\nhybrid_decrypt_backend = "age"\nage_identity = '
        + json.dumps(str(identity))
        + "\n"
    )
    home = root / "home"
    home.mkdir()
    env = dict(
        os.environ,
        HOME=str(home),
        XDG_CACHE_HOME=str(home / ".cache"),
        SHINE_CONFIG_DIR=str(config),
        PATH=str(tools),
        TRACE=str(trace),
        WORKSPACE=str(workspace),
        COUNTER=str(counter),
        CASE="",
    )
    env.pop("TOKEN", None)
    env.pop("SHINE_PRESETS", None)

    def policy(backend="hybrid", files=None):
        files = files or ["source.toml"]
        workspace.write_text(
            'version = 2\n[env]\ndefault_mode = "test"\nfiles = '
            + json.dumps(files)
            + "\n[env.encryption]\nbackend = "
            + json.dumps(backend)
            + '\ngpg_recipients = ["'
            + "A" * 40
            + '"]\nage_recipients = ["age1test"]\n'
        )

    def call(args, ok=True, **changes):
        result = subprocess.run(
            [binary] + args, cwd=root, env=dict(env, **changes), capture_output=True
        )
        assert (result.returncode == 0) == ok, (args, result.stderr.decode())
        return result

    def seal(ok=True, **changes):
        return call(
            ["env", "secret", "seal", "--workspace", str(workspace)], ok, **changes
        )

    source = root / "source.toml"
    env["SOURCE"] = str(source)
    original = '[plain]\nPLAIN="public"\n[secret]\nTOKEN="fixture secret"\n'
    for mode in ["fail-age", "edit-workspace", "edit-source", "remove-source"]:
        policy()
        source.write_text(original)
        result = seal(False, CASE=mode)
        assert b"current file" in result.stderr and b"not updated" in result.stderr
        if mode == "edit-source":
            assert source.read_text() == "# external source edit\n"
        elif mode == "remove-source":
            assert not source.exists()
        else:
            assert source.read_text() == original
    policy(files=["one.toml", "two.toml", "three.toml"])
    for name in ["one", "two", "three"]:
        (root / (name + ".toml")).write_text(original)
    result = seal(False, CASE="multi")
    assert b"1 completed" in result.stderr and b"1 unprocessed" in result.stderr
    assert "hybrid:" in (root / "one.toml").read_text()
    assert (root / "two.toml").read_text() == original and (
        root / "three.toml"
    ).read_text() == original
    policy()
    source.write_text(original)
    seal()
    sealed = source.read_text()
    data = re.search(r'data = "(hybrid:[A-Za-z0-9+/=]+)"', sealed).group(1)
    trace.write_text("")
    # An age-only reader builds a local age cache, ignoring the legacy cache.
    (tools / "gpg").rename(tools / "gpg-disabled")
    policy("gpg")
    cache_base = (
        home / "Library" / "Caches" if sys.platform == "darwin" else home / ".cache"
    )
    cache = (
        cache_base
        / "shine"
        / "projects"
        / hashlib.sha256(str(root.resolve()).encode()).hexdigest()
        / "env-test.toml"
    )
    cache.parent.mkdir(parents=True)

    def forge_cache():
        digest = hashlib.sha256(
            (1).to_bytes(4, "little")
            + b"test"
            + workspace.read_bytes()
            + str(source).encode()
            + source.read_bytes()
            + str(workspace).encode()
        ).hexdigest()
        payload = b'version = 1\n[values]\nTOKEN = "WRONG cached value"\n'
        cache.write_text(
            "version = 1\nproject_root = "
            + json.dumps(str(root))
            + '\n[modes.test]\ninput_hash = "sha256:'
            + digest
            + '"\nkeys = ["TOKEN"]\ndata = "age:'
            + base64.b64encode(payload).decode()
            + '"\n'
        )

    forge_cache()
    cache_original = cache.read_bytes()
    for _ in range(2):
        result = call(
            [
                "env",
                "run",
                "--workspace",
                str(workspace),
                "--mode",
                "test",
                "--",
                "/bin/sh",
                "-c",
                'printf %s "$TOKEN"',
            ]
        )
        assert result.stdout == b"fixture secret", (
            "unexpected fixture output length",
            len(result.stdout),
        )
        assert result.stderr == b"", result.stderr.decode()
    assert cache.read_bytes() == cache_original
    assert trace.read_text().splitlines() == [
        "decrypt-age",
        "encrypt-age",
        "decrypt-age",
    ]
    # A personal preference overrides the global age selection on a GPG-only reader.
    (tools / "age").rename(tools / "age-disabled")
    (tools / "gpg-disabled").rename(tools / "gpg")
    current = (config / "config.toml").read_text()
    local_config = root / "shine.config.local.toml"
    local_config.write_text('hybrid_decrypt_backend = "gpg"\n')
    trace.write_text("")
    for _ in range(2):
        result = call(
            [
                "env",
                "run",
                "--workspace",
                str(workspace),
                "--mode",
                "test",
                "--",
                "/bin/sh",
                "-c",
                'printf %s "$TOKEN"',
            ]
        )
        assert result.stdout == b"fixture secret" and result.stderr == b""
    assert trace.read_text().splitlines() == [
        "decrypt-gpg",
        "encrypt-gpg",
        "decrypt-gpg",
    ]
    assert cache.read_bytes() == cache_original
    local_config.unlink()
    (tools / "gpg").rename(tools / "gpg-disabled")
    (tools / "age-disabled").rename(tools / "age")
    # The age cache survives switching to GPG and back. Cancellation is terminal.
    run_args = [
        "env",
        "run",
        "--workspace",
        str(workspace),
        "--mode",
        "test",
        "--",
        "/bin/sh",
        "-c",
        'printf %s "$TOKEN"',
    ]
    trace.write_text("")
    call(run_args, False, CASE="fail-decrypt")
    assert trace.read_text().splitlines() == ["decrypt-age"]
    trace.write_text("")
    assert call(run_args).stdout == b"fixture secret"
    assert trace.read_text().splitlines() == ["decrypt-age"]

    # Two sources decrypt once each on a miss, but only once total on a hit.
    second = root / "second.toml"
    second.write_text(sealed)
    policy("hybrid", ["source.toml", "second.toml"])
    trace.write_text("")
    assert call(run_args).stdout == b"fixture secret"
    assert trace.read_text().splitlines() == [
        "decrypt-age",
        "decrypt-age",
        "encrypt-age",
    ]
    trace.write_text("")
    assert call(run_args).stdout == b"fixture secret"
    assert trace.read_text().splitlines() == ["decrypt-age"]

    # Workspace changes invalidate the cache; only the selected encryption tool runs.
    workspace.write_text(workspace.read_text() + "\n# changed workspace\n")
    trace.write_text("")
    assert call(run_args).stdout == b"fixture secret"
    assert trace.read_text().splitlines() == [
        "decrypt-age",
        "decrypt-age",
        "encrypt-age",
    ]

    # Cache writes are private and encryption failure does not block execution.
    local_age_cache = cache.with_name("env-test-hybrid-local-age.toml")
    assert local_age_cache.stat().st_mode & 0o777 == 0o600
    workspace.write_text(workspace.read_text() + "\n# force cache miss\n")
    trace.write_text("")
    result = call(run_args, CASE="fail-age")
    assert result.stdout == b"fixture secret"
    assert b"could not update environment cache" in result.stderr
    assert trace.read_text().splitlines() == [
        "decrypt-age",
        "decrypt-age",
        "encrypt-age",
    ]

    # Missing workspace recipients never fall back to the global recipient list.
    policy("gpg")
    workspace.write_text(
        workspace.read_text().replace(
            'age_recipients = ["age1test"]', "age_recipients = []"
        )
    )
    (config / "config.toml").write_text('age_recipients = ["age1global"]\n' + current)
    trace.write_text("")
    assert call(run_args).stdout == b"fixture secret"
    assert trace.read_text().splitlines() == ["decrypt-age"]
    (config / "config.toml").write_text(current)

    # A corrupt tagged payload cannot be hidden by cache; dry-run and default exports never decrypt.
    source.write_text('[secret]\nTOKEN=true\n[payload]\ndata="hybrid:bad"\n')
    trace.write_text("")
    forge_cache()
    call(
        [
            "env",
            "run",
            "--workspace",
            str(workspace),
            "--mode",
            "test",
            "--",
            "/usr/bin/true",
        ],
        False,
    )
    for flags in [[], ["--include-secrets", "--dry-run"]]:
        call(
            [
                "env",
                "workspace",
                "export",
                "--workspace",
                str(workspace),
                "--mode",
                "test",
                "--format",
                "dotenv",
                "--output",
                str(root / "export.env"),
                "--force",
            ]
            + flags
        )
    assert trace.read_text() == ""
    source.write_text(sealed)
    # Exact decrypt stdout equals the underlying serialized secret payload, including its newline.
    current = (config / "config.toml").read_text()
    (config / "config.toml").write_text(
        current.replace("[env]\n", "[env]\nTOKEN_SECRET=" + json.dumps(data) + "\n", 1)
    )
    result = call(["env", "secret", "decrypt", "TOKEN_SECRET"])
    expected = 'version = 1\n\n[values]\nTOKEN = "fixture secret"\n'.encode()
    assert result.stdout == expected, repr(result.stdout)
    print(
        "PASS: encryption failure; workspace/source edit and removal; partial multi-file reporting; age-only/GPG-only local caches; personal preference; cache hit/miss and cancellation; malformed hybrid rejection; dry-run/default export boundaries; exact decrypt stdout"
    )
