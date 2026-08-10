import {readdir} from 'node:fs/promises';
import {fileURLToPath} from 'node:url';
import path from 'node:path';

const websiteDir = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const englishRoot = path.resolve(websiteDir, '../docs/manual');
const chineseRoot = path.join(
  websiteDir,
  'i18n/zh-Hans/docusaurus-plugin-content-docs/current',
);

async function contentFiles(root, current = root) {
  const entries = await readdir(current, {withFileTypes: true});
  const nested = await Promise.all(
    entries
      .filter((entry) => !entry.name.startsWith('.'))
      .map(async (entry) => {
        const absolute = path.join(current, entry.name);
        if (entry.isDirectory()) {
          return contentFiles(root, absolute);
        }
        if (entry.name.endsWith('.md') || entry.name === '_category_.json') {
          return [path.relative(root, absolute)];
        }
        return [];
      }),
  );
  return nested.flat();
}

const english = new Set(await contentFiles(englishRoot));
const chinese = new Set(await contentFiles(chineseRoot));
const missingChinese = [...english].filter((file) => !chinese.has(file)).sort();
const missingEnglish = [...chinese].filter((file) => !english.has(file)).sort();

if (missingChinese.length || missingEnglish.length) {
  if (missingChinese.length) {
    console.error(`Missing zh-Hans content:\n${missingChinese.join('\n')}`);
  }
  if (missingEnglish.length) {
    console.error(`Missing English content:\n${missingEnglish.join('\n')}`);
  }
  process.exitCode = 1;
} else {
  console.log(`Locale content is aligned (${english.size} files per locale).`);
}
