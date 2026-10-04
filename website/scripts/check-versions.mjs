import {readdir, readFile} from 'node:fs/promises';
import {createRequire} from 'node:module';
import path from 'node:path';
import {fileURLToPath} from 'node:url';

const scriptPath = fileURLToPath(import.meta.url);

async function contentFiles(root, prefix = '') {
  const entries = await readdir(root, {withFileTypes: true});
  const files = await Promise.all(entries.filter((entry) => !entry.name.startsWith('.')).map(async (entry) => {
    const relative = path.join(prefix, entry.name);
    if (entry.isDirectory()) return contentFiles(path.join(root, entry.name), relative);
    return /\.mdx?$/.test(entry.name) || entry.name === '_category_.json' ? [relative] : [];
  }));
  return files.flat();
}

export async function checkVersions(websiteDir, docs) {
  const versions = JSON.parse(await readFile(path.join(websiteDir, 'versions.json'), 'utf8'));
  const errors = [];
  if (!Array.isArray(versions) || !versions.length ||
      versions.some((version) => typeof version !== 'string' || !/^(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(version))) {
    return ['versions.json must contain stable major.minor versions.'];
  }
  const majors = versions.map((version) => version.split('.')[0]);
  if (new Set(majors).size !== majors.length) {
    errors.push('Keep only one stable documentation version per major.');
  }
  const sorted = [...versions].sort((a, b) => {
    const [aMajor, aMinor] = a.split('.').map(Number);
    const [bMajor, bMinor] = b.split('.').map(Number);
    return bMajor - aMajor || bMinor - aMinor;
  });
  if (JSON.stringify(sorted) !== JSON.stringify(versions)) {
    errors.push('versions.json must list newest versions first.');
  }
  function compare(label, actual, expected) {
    const actualSet = new Set(actual);
    const expectedSet = new Set(expected);
    for (const item of expectedSet) {
      if (!actualSet.has(item)) errors.push(`${label}: missing ${item}`);
    }
    for (const item of actualSet) {
      if (!expectedSet.has(item)) errors.push(`${label}: unexpected ${item}`);
    }
  }
  compare('docs.versions', Object.keys(docs.versions ?? {}), ['current', ...versions]);
  if (docs.lastVersion !== versions[0] || docs.versions?.[versions[0]]?.path !== '') {
    errors.push('The latest stable version must be lastVersion and use the root path.');
  }
  if (docs.versions?.current?.path !== 'next') {
    errors.push('The current documentation must use the next path.');
  }
  for (const version of versions.slice(1)) {
    if (docs.versions?.[version]?.path !== version) {
      errors.push(`Archived major version ${version} must use its version as its path.`);
    }
  }
  const chinese = 'i18n/zh-Hans/docusaurus-plugin-content-docs';
  const inventories = [
    ['versioned_docs', versions.map((v) => `version-${v}`), true],
    ['versioned_sidebars', versions.map((v) => `version-${v}-sidebars.json`), false],
    [chinese, versions.map((v) => `version-${v}`), true],
    [chinese, versions.map((v) => `version-${v}.json`), false],
  ];
  for (const [directory, expected, directories] of inventories) {
    const entries = await readdir(path.join(websiteDir, directory), {withFileTypes: true});
    const actual = entries.filter((entry) => entry.name.startsWith('version-') &&
      (directories ? entry.isDirectory() : !entry.isDirectory())).map((entry) => entry.name);
    compare(`${directory} (${directories ? 'directories' : 'files'})`, actual, expected);
  }
  if (!errors.length) {
    for (const version of versions) {
      const englishFiles = await contentFiles(path.join(websiteDir, 'versioned_docs', `version-${version}`));
      const chineseFiles = await contentFiles(path.join(websiteDir, chinese, `version-${version}`));
      compare(`Version ${version} Chinese content`, chineseFiles, englishFiles);
    }
  }
  return errors;
}

if (process.argv[1] && path.resolve(process.argv[1]) === scriptPath) {
  try {
    const websiteDir = path.dirname(path.dirname(scriptPath));
    const require = createRequire(import.meta.url);
    const config = require(path.join(websiteDir, 'docusaurus.config.js'));
    const docs = config.presets.find((preset) => Array.isArray(preset) && preset[0] === 'classic')[1].docs;
    const errors = await checkVersions(websiteDir, docs);
    if (errors.length) {
      console.error(errors.join('\n'));
      process.exitCode = 1;
    } else {
      console.log('Documentation versions, configuration, and bilingual snapshots are aligned.');
    }
  } catch (error) {
    console.error(`Documentation version check failed: ${error.message}`);
    process.exitCode = 1;
  }
}
