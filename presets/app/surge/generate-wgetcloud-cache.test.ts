import { expect, onTestFinished, test } from "bun:test";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

const generatorPath = join(import.meta.dir, "generate-wgetcloud-cache.ts");

function runGenerator(path: string, profilesDirectory?: string) {
  const args = profilesDirectory === undefined ? [generatorPath] : [
    "--eval",
    `import { runCacheGenerator } from ${JSON.stringify(generatorPath)};
     await runCacheGenerator(process.env.W_GET_CLOUD_SURGE_CACHE_SOURCE, ${JSON.stringify(profilesDirectory)});`,
  ];
  return Bun.spawnSync({
    cmd: [process.execPath, ...args],
    env: { W_GET_CLOUD_SURGE_CACHE_SOURCE: path },
    stdout: "pipe",
    stderr: "pipe",
  });
}

async function cacheDirectory() {
  const directory = await mkdtemp(join(tmpdir(), "shine-wgetcloud-"));
  onTestFinished(() => rm(directory, { force: true, recursive: true }));
  return directory;
}

function expectSuccess(result: ReturnType<typeof runGenerator>, output: string) {
  expect(result.exitCode).toBe(0);
  expect(result.stdout.toString()).toBe(output);
  expect(result.stderr.toString()).toBe("");
}

function expectFailure(result: ReturnType<typeof runGenerator>, code: string) {
  expect(result.exitCode).toBe(1);
  expect(result.stdout.toString()).toBe("");
  expect(result.stderr.toString()).toBe(`shine-generator-diagnostic-v1:${code}\n`);
}

test.each([
  ["with only a Proxy section", []],
  ["with a renamed policy group", ["[Proxy Group] # provider groups", "Provider Nodes = select, Tokyo, DIRECT"]],
  ["with the original policy group", ["[Proxy Group]", "🚀 节点选择 = select, Tokyo, DIRECT"]],
] as const)("imports all wGetCloud proxies %s", async (_scenario, groupLines) => {
  const directory = await cacheDirectory();
  const path = join(directory, "local cache.conf");
  const first = 'Osaka, JP = trojan, test.invalid, 443, password="example, one"';
  const second = "Tokyo = ss, test.invalid, 443, password=example";
  await writeFile(path, [
    "\ufeff[Proxy]",
    "# Provider proxies",
    first,
    second,
    ...groupLines,
  ].join("\r\n"));
  expectSuccess(runGenerator(path), `${first}\n${second}\n`);
});

test("resolves a named Surge configuration to the cache of its exact subscription reference", async () => {
  const directory = await cacheDirectory();
  const managed = join(directory, ".managed");
  const profile = join(directory, "WgetCloud.conf");
  const reference = "https://Provider.Example:443/link/test?token=fake%2fvalue";
  const cache = join(managed, "f0deaab2b6108f06d6fd889d95bcb7fe.conf");
  const proxy = "Offline = trojan, cache.invalid, 443, password=example";
  const profileText = `[Proxy]\n#!include ${reference}\n[Proxy Group]\n#!include "${reference}" # same subscription\n[Rule] # unrelated section\n#!include https://rules.invalid/list\n`;
  await mkdir(managed);
  await writeFile(profile, profileText);
  await writeFile(
    join(managed, "unrelated.conf"),
    "[Proxy]\nOther = trojan, other.invalid, 443\n[Proxy Group]\n🚀 节点选择 = select, Other\n",
  );

  expectFailure(runGenerator("WgetCloud.conf", directory), "surge-profile-cache-missing");

  await writeFile(cache, `[Proxy]\n${proxy}\n[Proxy Group]\n🚀 节点选择 = select, Offline\n`);
  expectSuccess(runGenerator("WgetCloud.conf", directory), `${proxy}\n`);

  for (const groupReference of [
    "https://other.invalid/link",
    `${reference}, https://other.invalid/link`,
  ]) {
    await writeFile(profile, `[Proxy]\n#!include ${reference}\n[General]\n[Proxy Group] # provider groups\n#!include ${groupReference}\n`);
    expectFailure(runGenerator("WgetCloud.conf", directory), "surge-profile-reference-ambiguous");
  }
});

test("failed cache reads and invalid Proxy sections produce no output or sensitive diagnostics", async () => {
  const directory = await cacheDirectory();
  const path = join(directory, "private-cache-location.conf");
  expectFailure(runGenerator(path), "cache-read-failed");

  const validProxy = "Known = ss, test.invalid, 443, password=secret";
  for (const lines of [
    ["[Proxy Group]", "Provider Nodes = select, DIRECT"],
    ["[Proxy]", "# No proxies"],
    ["[Proxy]", validProxy, "Broken"],
    ["[Proxy]", validProxy, "Other ="],
    ["[Proxy]", validProxy, validProxy],
    ["[Proxy]", validProxy, "#!include more-proxies.conf"],
  ]) {
    await writeFile(path, lines.join("\n"));
    expectFailure(runGenerator(path), "invalid-surge-cache");
  }
});
