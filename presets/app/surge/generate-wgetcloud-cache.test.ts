import { expect, test } from "bun:test";
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

test("generates the wGetCloud node list offline from an explicitly configured cache file", async () => {
  const directory = await mkdtemp(join(tmpdir(), "shine-wgetcloud-"));
  try {
    const path = join(directory, "local cache.conf");
    const first = 'Osaka, JP = trojan, test.invalid, 443, password="example, one"';
    const second = "Tokyo = ss, test.invalid, 443, password=example";
    await writeFile(path, [
      "\ufeff[General]",
      "loglevel = notify",
      "[Proxy]",
      first,
      second,
      "Unselected = ss, test.invalid, 443, password=unused",
      "[Proxy Group]",
      '🚀 节点选择 = select, Tokyo, DIRECT, "Osaka, JP", Tokyo',
      "[Rule]",
      "FINAL,🚀 节点选择",
    ].join("\r\n"));
    const result = runGenerator(path);
    expect(result.exitCode).toBe(0);
    expect(result.stdout.toString()).toBe(`${second}\n${first}\n`);
    expect(result.stderr.toString()).toBe("");
  } finally {
    await rm(directory, { force: true, recursive: true });
  }
});

test("resolves a named Surge configuration to the cache of its exact subscription reference", async () => {
  const directory = await mkdtemp(join(tmpdir(), "shine-wgetcloud-"));
  try {
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

    const missing = runGenerator("WgetCloud.conf", directory);
    expect(missing.exitCode).toBe(1);
    expect(missing.stdout.toString()).toBe("");
    expect(missing.stderr.toString()).toBe("shine-generator-diagnostic-v1:surge-profile-cache-missing\n");

    await writeFile(cache, `[Proxy]\n${proxy}\n[Proxy Group]\n🚀 节点选择 = select, Offline\n`);
    const offline = runGenerator("WgetCloud.conf", directory);
    expect(offline.exitCode).toBe(0);
    expect(offline.stdout.toString()).toBe(`${proxy}\n`);
    expect(offline.stderr.toString()).toBe("");

    for (const groupReference of [
      "https://other.invalid/link",
      `${reference}, https://other.invalid/link`,
    ]) {
      await writeFile(profile, `[Proxy]\n#!include ${reference}\n[General]\n[Proxy Group] # provider groups\n#!include ${groupReference}\n`);
      const ambiguous = runGenerator("WgetCloud.conf", directory);
      expect(ambiguous.exitCode).toBe(1);
      expect(ambiguous.stdout.toString()).toBe("");
      expect(ambiguous.stderr.toString()).toBe("shine-generator-diagnostic-v1:surge-profile-reference-ambiguous\n");
    }
  } finally {
    await rm(directory, { force: true, recursive: true });
  }
});

test("failed cache reads and invalid node selections produce no output or sensitive diagnostics", async () => {
  const directory = await mkdtemp(join(tmpdir(), "shine-wgetcloud-"));
  try {
    const path = join(directory, "private-cache-location.conf");
    const missing = runGenerator(path);
    expect(missing.exitCode).toBe(1);
    expect(missing.stdout.toString()).toBe("");
    expect(missing.stderr.toString()).toBe("shine-generator-diagnostic-v1:cache-read-failed\n");

    for (const members of ["Known, Missing", '"Known']) {
      await writeFile(path, [
        "[Proxy]",
        "Known = ss, test.invalid, 443, password=secret",
        "[Proxy Group]",
        `🚀 节点选择 = select,${members}`,
      ].join("\n"));
      const invalid = runGenerator(path);
      expect(invalid.exitCode).toBe(1);
      expect(invalid.stdout.toString()).toBe("");
      expect(invalid.stderr.toString()).toBe("shine-generator-diagnostic-v1:invalid-surge-cache\n");
    }
  } finally {
    await rm(directory, { force: true, recursive: true });
  }
});
