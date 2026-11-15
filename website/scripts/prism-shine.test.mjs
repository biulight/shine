import assert from 'node:assert/strict';
import {createRequire} from 'node:module';
import test from 'node:test';
import {Prism} from 'prism-react-renderer';
import includeShine from '../src/prism-shine.js';

const require = createRequire(import.meta.url);
const presetRequire = createRequire(require.resolve('@docusaurus/preset-classic'));
const themeRequire = createRequire(presetRequire.resolve('@docusaurus/theme-classic'));
globalThis.Prism = Prism;
for (const language of ['bash', 'powershell']) {
  themeRequire(`prismjs/components/prism-${language}`);
}
delete globalThis.Prism;
includeShine(Prism);

function commands(language, source) {
  const found = [];
  function visit(tokens) {
    for (const token of tokens) {
      if (typeof token === 'string') continue;
      if (token.type === 'shine-command') found.push(token.content);
      if (Array.isArray(token.content)) visit(token.content);
    }
  }
  visit(Prism.tokenize(source, Prism.languages[language]));
  return found;
}

for (const language of ['bash', 'sh', 'shell', 'powershell']) {
  test(`${language}: command positions retain dedicated highlighting`, () => {
    for (const source of [
      'shine app list', '  shine --help', 'sudo shine app list',
      'echo ok && shine --version', 'echo ok | shine env exec',
      'echo ok; shine --help', 'echo ok\nshine --help',
    ]) assert.deepEqual(commands(language, source), ['shine'], source);
  });

  test(`${language}: strings, comments, arguments and filenames are not commands`, () => {
    for (const source of [
      '# shine --help', '# echo ok; shine --help',
      'echo "shine --help"', "echo 'shine --help'",
      'echo "ok; shine --help"', "echo 'ok | shine --help'",
      'echo "first\nshine --help\nlast"',
      "echo 'first\nshine --help\nlast'",
      'echo "text" shine', 'echo shine', 'echo ok; echo shine',
      'shine.toml', 'shine-other --help', './shine --help',
    ]) assert.deepEqual(commands(language, source), [], source);
    assert.deepEqual(commands(language, 'shine --help # shine --version'), ['shine']);
    assert.deepEqual(commands(language, 'echo "shine"; shine --help'), ['shine']);
    assert.deepEqual(commands(language, 'shine --help && shine --version'), ['shine', 'shine']);
  });
}

test('Bash: assignment prefixes, wrappers and heredocs', () => {
  for (const source of ['SHINE_CONFIG_DIR=/tmp/demo shine app list', 'command shine --help', 'exec shine --help']) {
    assert.deepEqual(commands('bash', source), ['shine']);
  }
  for (const source of ['cat <<EOF\nshine --help\nEOF', "cat <<'EOF'\nshine --help\nEOF"]) {
    assert.deepEqual(commands('bash', source), []);
  }
});

test('escaped separators, continued arguments and Bash arrays are not command positions', () => {
  for (const [language, escape] of [['bash', '\\'], ['powershell', '`']]) {
    for (const source of [
      `echo ${escape}; shine`, `echo ${escape}| shine`,
      `echo ${escape}\nshine`, `echo ${escape}\r\n  shine`,
    ]) assert.deepEqual(commands(language, source), [], source);
  }
  assert.deepEqual(commands('bash', 'values=(shine)'), []);
  assert.deepEqual(commands('bash', '(shine --help)'), ['shine']);
});

test('PowerShell: executable casing, invocation operator, block comments and here-strings', () => {
  assert.deepEqual(commands('powershell', '& SHINE.exe --help'), ['SHINE.exe']);
  assert.deepEqual(commands('powershell', 'if ($true) { shine --help }'), ['shine']);
  assert.deepEqual(commands('powershell', '$exportCode = shine env secret export MY_TOKEN --as API_TOKEN'), ['shine']);
  for (const source of ['<#\nshine --help\n#>', '@"\nshine --help\n"@', "@'\nshine --help\n'@", '& "shine" --help']) {
    assert.deepEqual(commands('powershell', source), [], source);
  }
});
