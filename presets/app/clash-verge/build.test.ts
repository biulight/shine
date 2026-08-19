import { afterEach, describe, expect, test } from "bun:test";
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

import {
  installPayload,
  providerRefreshUrl,
  refreshProviders,
  renderPayload,
  resolveBoundFiles,
  type BoundFiles,
} from "./build";

const tempDirs: string[] = [];
function tempDir(): string {
  const dir = mkdtempSync(join(tmpdir(), "shine-clash-verge-"));
  tempDirs.push(dir);
  return dir;
}
afterEach(() => {
  for (const dir of tempDirs.splice(0)) rmSync(dir, { recursive: true, force: true });
});

function profilesFixture(file = "merge.yaml"): string {
  return `current: remote-id
items:
  - { uid: merge-id, type: merge, file: ${file} }
  - { uid: rules-id, type: rules, file: rules.yaml }
  - { uid: proxies-id, type: proxies, file: proxies.yaml }
  - { uid: groups-id, type: groups, file: groups.yaml }
  - uid: remote-id
    type: remote
    file: subscription.yaml
    option:
      merge: merge-id
      rules: rules-id
      proxies: proxies-id
      groups: groups-id
`;
}

describe("resolveBoundFiles", () => {
  test("resolves every enhancement bound to the active subscription", () => {
    const dir = tempDir();
    const profiles = join(dir, "profiles.yaml");
    writeFileSync(profiles, profilesFixture());

    expect(resolveBoundFiles(profiles)).toEqual({
      merge: join(dir, "profiles", "merge.yaml"),
      rules: join(dir, "profiles", "rules.yaml"),
      proxies: join(dir, "profiles", "proxies.yaml"),
      groups: join(dir, "profiles", "groups.yaml"),
    });
  });

  test("does not fall back to global enhancements when a binding is absent", () => {
    const dir = tempDir();
    const profiles = join(dir, "profiles.yaml");
    writeFileSync(profiles, profilesFixture().replace("      rules: rules-id\n", ""));
    expect(resolveBoundFiles(profiles)).toBeNull();
  });

  test("rejects a bound filename outside the profiles directory", () => {
    const dir = tempDir();
    const profiles = join(dir, "profiles.yaml");
    writeFileSync(profiles, profilesFixture("../profiles.yaml"));
    expect(resolveBoundFiles(profiles)).toBeNull();
  });
});

test("renderPayload splits the composite source into CVR 2.x editor documents", () => {
  const dir = tempDir();
  const source = join(dir, "merge.yaml");
  writeFileSync(
    source,
    `proxies:
  - { name: LAN-SOCKS, type: socks5, server: 127.0.0.1, port: 1080 }
proxy-groups:
  - { name: Local, type: select, proxies: [DIRECT] }
rule-providers:
  lan: { type: http, url: https://example.test/lan.list }
prepend-rules:
  - RULE-SET,lan,Local
`,
  );

  const rendered = renderPayload(source);
  expect(rendered.providers).toEqual(["lan"]);
  expect(Bun.YAML.parse(rendered.payload.merge)).toEqual({
    "rule-providers": { lan: { type: "http", url: "https://example.test/lan.list" } },
  });
  expect(Bun.YAML.parse(rendered.payload.rules)).toEqual({
    prepend: ["RULE-SET,lan,Local"],
    append: [],
    delete: [],
  });
  expect((Bun.YAML.parse(rendered.payload.proxies) as { prepend: unknown[] }).prepend).toHaveLength(1);
  expect((Bun.YAML.parse(rendered.payload.groups) as { prepend: unknown[] }).prepend).toHaveLength(1);
});

test("renderPayload derives arbitrary provider names from the composite source", () => {
  const dir = tempDir();
  const source = join(dir, "merge.yaml");
  writeFileSync(
    source,
    `rule-providers:
  renamed-provider: { type: file, path: ./renamed.list }
  office / lab: { type: http, url: https://example.test/office.list }
`,
  );

  expect(renderPayload(source).providers).toEqual(["renamed-provider", "office / lab"]);
});

test("HTTP mode remains remote and does not reference built-in local files", () => {
  const dir = tempDir();
  const source = join(dir, "merge.yaml");
  writeFileSync(
    source,
    `rule-providers:
  lan:
    type: http
    proxy: DIRECT
    behavior: classical
    format: text
    interval: 86400
    url: https://rules.example.com/lan.list
    path: ./ruleset/shine-remote-lan.list
`,
  );

  const merge = Bun.YAML.parse(renderPayload(source).payload.merge) as {
    "rule-providers": Record<string, Record<string, unknown>>;
  };
  expect(merge["rule-providers"].lan).toEqual({
    type: "http",
    proxy: "DIRECT",
    behavior: "classical",
    format: "text",
    interval: 86400,
    url: "https://rules.example.com/lan.list",
    path: "./ruleset/shine-remote-lan.list",
  });
});

test("renderPayload treats missing, null, or empty rule-providers as empty", () => {
  const dir = tempDir();
  const missing = join(dir, "missing.yaml");
  const nullMapping = join(dir, "null.yaml");
  const emptyMapping = join(dir, "empty.yaml");
  writeFileSync(missing, "dns: {}\n");
  writeFileSync(nullMapping, "rule-providers: null\n");
  writeFileSync(emptyMapping, "rule-providers: {}\n");

  expect(renderPayload(missing).providers).toEqual([]);
  expect(renderPayload(nullMapping).providers).toEqual([]);
  expect(renderPayload(emptyMapping).providers).toEqual([]);
});

test.each(["[]", "provider-name"])("renderPayload rejects non-mapping rule-providers: %s", (value) => {
  const dir = tempDir();
  const source = join(dir, "merge.yaml");
  writeFileSync(source, `rule-providers: ${value}\n`);

  expect(() => renderPayload(source)).toThrow("'rule-providers' must be a YAML mapping");
});

test("providerRefreshUrl encodes provider names as one path segment", () => {
  expect(providerRefreshUrl("http://127.0.0.1:9090", "office / lab")).toBe(
    "http://127.0.0.1:9090/providers/rules/office%20%2F%20lab",
  );
});

test("refreshProviders skips an empty provider set", async () => {
  expect(await refreshProviders([])).toBe("skipped");
});

test("installPayload is idempotent and marks every managed copy", () => {
  const dir = tempDir();
  const targetDir = join(dir, "profiles");
  mkdirSync(targetDir);
  const targets = Object.fromEntries(
    ["merge", "rules", "proxies", "groups"].map((kind) => [kind, join(targetDir, `${kind}.yaml`)]),
  ) as BoundFiles;
  const editor = "prepend: []\nappend: []\ndelete: []\n";
  const payload = { merge: "{}\n", rules: editor, proxies: editor, groups: editor };

  expect(installPayload(payload, targets)).toBe("changed");
  for (const [kind, target] of Object.entries(targets)) {
    expect(readFileSync(target, "utf8")).toStartWith(
      `# Managed by shine (app/clash-verge, ${kind}).`,
    );
  }
  // CVR rewrites comments/formatting on save; semantic equality must remain current.
  writeFileSync(targets.rules, "delete: []\n# rewritten by CVR\nappend: []\nprepend: []\n");
  expect(installPayload(payload, targets)).toBe("current");
});

test("the example exposes the composite source keys", () => {
  const example = readFileSync(join(import.meta.dir, "merge.yaml"), "utf8");
  expect(example).toContain("# proxies:");
  expect(example).toContain("# proxy-groups:");
  expect(example).toContain("# prepend-rules:");
  expect(example).toContain("name: LAN Network, type: select");
  expect(example).toContain("name: LAN PROXY, type: select");
  expect(example).toContain("type: file, behavior: classical, format: text");
  for (const name of ["lan.list", "lan-socks.list", "other-direct.list"]) {
    expect(readFileSync(join(import.meta.dir, "rules", name), "utf8")).toStartWith("#");
    expect(example).toContain(`./ruleset/shine-source/${name}`);
  }
  expect(example).toContain("http://127.0.0.1:8080/rules/lan.list");
  expect(example).toContain("https://rules.example.com/surge/lan.list");
  expect(example).toContain('"+.corp.example": 192.0.2.53');
  expect(example.match(/^#   (?:lan|lan-socks|other-direct):.*proxy: DIRECT/gm)).toHaveLength(6);
  expect(example).not.toContain("exclude-filter:");
  expect(example).not.toContain("surge.biulight.internal");
});
