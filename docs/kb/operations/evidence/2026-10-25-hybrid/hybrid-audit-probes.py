#!/usr/bin/env python3
"""Check the 2026-10-25 audit fixes using disposable synthetic fixtures.

Run from the repository root with the current built Shine binary as argv[1].
Reuses the reviewed CLI regression fixture setup, not its test assertions.
The fake public-key wrappers contain plaintext and must never handle real secrets.
Pass --expect-vulnerable after the binary to reproduce the original audit baseline.
"""

from pathlib import Path
import sys

expect_vulnerable = "--expect-vulnerable" in sys.argv[2:]
repo = Path(__file__).resolve().parents[5]
fixture = (repo / "scripts/test-hybrid-secrets.py").read_text()
boundary = '    source = root / "source.toml"'
assert boundary in fixture, "CLI fixture layout changed; review before running"
setup = fixture.split(boundary, 1)[0]

probes = r"""
    source = root / "source.toml"
    policy()
    source.write_text('[secret]\nTOKEN="audit fixture"\n')
    seal()
    data = re.search(r'data = "(hybrid:[A-Za-z0-9+/=]+)"', source.read_text()).group(1)
    first = root / "first.toml"
    second = root / "second.toml"
    policy(files=["first.toml", "second.toml"])
    x = 'secret = {TOKEN = true}\n#'
    y = '\n[payload]\ndata = ' + json.dumps(data) + '\n'
    first.write_text('#')
    second.write_text(x + str(second) + y)

    def digest():
        return hashlib.sha256(
            (1).to_bytes(4, 'little') + b'test' + workspace.read_bytes()
            + str(first).encode() + first.read_bytes()
            + str(second).encode() + second.read_bytes()
            + str(workspace).encode()
        ).hexdigest()

    run_args = [
        'env', 'run', '--workspace', str(workspace), '--mode', 'test', '--',
        '/bin/sh', '-c', 'printf %s "${TOKEN-ABSENT}"',
    ]
    original_hash = digest()
    assert call(run_args).stdout == b'audit fixture'
    first.write_text('#' + str(second) + x)
    second.write_text(y)
    assert digest() == original_hash
    trace.write_text('')
    cached = call(run_args, expect_vulnerable)
    print('legacy_input_hash_equal:', digest() == original_hash)
    print('cached_run_releases_removed_TOKEN:', cached.stdout == b'audit fixture')
    print('cached_run_trace:', trace.read_text().strip())
    cache_base = home / 'Library' / 'Caches' if sys.platform == 'darwin' else home / '.cache'
    cache_paths = list(cache_base.rglob('env-test-hybrid-local-age.toml'))
    assert len(cache_paths) == 1
    cache_paths[0].unlink()
    fresh = call(run_args, False)
    print('without_cache_exit:', fresh.returncode)
    print('without_cache_key_mismatch:', b'secret key list does not match encrypted payload' in fresh.stderr)
    assert (cached.stdout == b'audit fixture') == expect_vulnerable
    if not expect_vulnerable:
        assert cached.stdout == b''
        assert b'secret key list does not match encrypted payload' in cached.stderr
    assert b'secret key list does not match encrypted payload' in fresh.stderr

    original_config = (config / 'config.toml').read_text()
    assert '[env]' not in original_config, 'Fixture changed; review config injection'
    (config / 'config.toml').write_text(
        original_config + '\n[env]\nTOKEN_SECRET=' + json.dumps(data) + '\n'
    )
    (tools / 'age').write_text(script.replace(
        ' /bin/cat "$last"',
        " /usr/bin/head -c 70000 /dev/zero | /usr/bin/tr '\\000' D >&2\n /bin/cat \"$last\"",
    ))
    result = call(['env', 'secret', 'decrypt', 'TOKEN_SECRET'], expect_vulnerable)
    print('age_unwrap_exit:', result.returncode)
    print('raw_stderr_bytes:', len(result.stderr))
    print('unbounded_diagnostic_forwarded:', result.stderr == b'D' * 70000)
    if expect_vulnerable:
        assert result.stderr == b'D' * 70000
    else:
        assert result.stdout == b'' and b'D' * 32 not in result.stderr
        assert b'hybrid unwrap output exceeds limit' in result.stderr
"""

exec(compile(setup + probes, str(__file__), "exec"))
