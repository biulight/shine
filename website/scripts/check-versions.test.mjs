import assert from 'node:assert/strict';
import {mkdtemp, mkdir, rm, writeFile} from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import {checkVersions} from './check-versions.mjs';

const chinese = 'i18n/zh-Hans/docusaurus-plugin-content-docs';

async function fixture(t, versions = ['2.2', '1.8']) {
  const root = await mkdtemp(path.join(os.tmpdir(), 'shine-doc-versions-'));
  t.after(() => rm(root, {recursive: true, force: true}));
  await writeFile(path.join(root, 'versions.json'), JSON.stringify(versions));
  await mkdir(path.join(root, 'versioned_sidebars'), {recursive: true});
  const docs = {lastVersion: versions[0], versions: {current: {path: 'next'}}};
  for (const [index, version] of versions.entries()) {
    docs.versions[version] = {path: index === 0 ? '' : version};
    for (const directory of ['versioned_docs', chinese]) {
      await mkdir(path.join(root, directory, `version-${version}`), {recursive: true});
    }
    for (const file of [`versioned_sidebars/version-${version}-sidebars.json`, `${chinese}/version-${version}.json`]) {
      await writeFile(path.join(root, file), '{}');
    }
  }
  return {root, docs};
}

test('accepts one stable version per major with matching resources', async (t) => {
  const {root, docs} = await fixture(t);
  assert.deepEqual(await checkVersions(root, docs), []);
});

test('rejects multiple stable versions of the same major', async (t) => {
  const {root, docs} = await fixture(t, ['2.2', '2.1', '1.8']);
  assert.match((await checkVersions(root, docs)).join('\n'), /one stable documentation version per major/);
});

test('rejects unregistered snapshots, sidebars, and translation metadata', async (t) => {
  const {root, docs} = await fixture(t);
  for (const directory of ['versioned_docs', chinese]) {
    await mkdir(path.join(root, directory, 'version-2.1'));
  }
  await writeFile(path.join(root, 'versioned_sidebars/version-2.1-sidebars.json'), '{}');
  await writeFile(path.join(root, chinese, 'version-2.1.json'), '{}');
  const errors = await checkVersions(root, docs);
  assert.equal(errors.filter((error) => error.includes('unexpected version-2.1')).length, 4);
});

test('rejects missing Chinese snapshots and metadata', async (t) => {
  const {root, docs} = await fixture(t);
  await rm(path.join(root, chinese, 'version-2.2'), {recursive: true});
  await rm(path.join(root, chinese, 'version-2.2.json'));
  const errors = await checkVersions(root, docs);
  assert.ok(errors.some((error) => error.includes('missing version-2.2')));
  assert.ok(errors.some((error) => error.includes('missing version-2.2.json')));
});

test('rejects configuration drift and incorrect default routes', async (t) => {
  const {root, docs} = await fixture(t);
  delete docs.versions['1.8'];
  docs.versions['2.1'] = {path: '2.1'};
  docs.lastVersion = '2.1';
  docs.versions.current.path = 'dev';
  const errors = (await checkVersions(root, docs)).join('\n');
  assert.match(errors, /missing 1.8/);
  assert.match(errors, /unexpected 2.1/);
  assert.match(errors, /latest stable version/);
  assert.match(errors, /next path/);
});

test('rejects patch version entries', async (t) => {
  const {root, docs} = await fixture(t, ['2.2.1', '1.8']);
  assert.match((await checkVersions(root, docs)).join('\n'), /major.minor/);
});

test('rejects incomplete Chinese snapshot page inventory', async (t) => {
  const {root, docs} = await fixture(t);
  await writeFile(path.join(root, 'versioned_docs/version-2.2/installation.md'), '# Installation');
  assert.match((await checkVersions(root, docs)).join('\n'), /Version 2.2 Chinese content: missing installation.md/);
});

test('sorts version numbers numerically', async (t) => {
  const {root, docs} = await fixture(t, ['10.0', '2.2', '1.8']);
  assert.deepEqual(await checkVersions(root, docs), []);
  await writeFile(path.join(root, 'versions.json'), JSON.stringify(['2.2', '10.0', '1.8']));
  assert.match((await checkVersions(root, docs)).join('\n'), /newest versions first/);
});
