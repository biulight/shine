import { describe, expect, test } from "bun:test";
import {
  chmod,
  mkdtemp,
  readFile,
  rm,
  stat,
  symlink,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { applyProfileArtifact } from "./profile-artifact";

const buildScript = "build";
const unbuildScript = "unbuild";

async function runArtifact(
  action: "build" | "unbuild",
  profile: string,
) {
  return applyProfileArtifact(action, profile);
}

async function makeFixture(profileContent: string): Promise<{
  dir: string;
  profile: string;
}> {
  const dir = await mkdtemp(join(tmpdir(), "shine-surge-artifact-"));
  const profile = join(dir, "Profile.conf");
  await writeFile(profile, profileContent);
  for (const name of [
    "local-proxies.conf",
    "local-proxy-groups.conf",
    "local-rules.conf",
  ]) {
    await writeFile(join(dir, name), "# fixture\n");
  }
  return { dir, profile };
}

describe("Surge profile artifact", () => {
  test("include apply/remove preserves a BOM and leaves General unchanged", async () => {
    const original = "\uFEFF[General]\r\nskip-proxy = localhost\r\n[Proxy]\r\n#!include proxies.conf\r\n[Proxy Group]\r\n#!include groups.conf\r\n[Rule]\r\n#!include rules.conf\r\n";
    const fixture = await makeFixture(original);
    try {
      await runArtifact("build", fixture.profile);
      const applied = await readFile(fixture.profile, "utf8");
      expect(applied.startsWith("\uFEFF[General]\r\nskip-proxy = localhost\r\n")).toBe(true);
      expect((await runArtifact("build", fixture.profile)).changed).toBe(false);
      await runArtifact("unbuild", fixture.profile);
      expect(await readFile(fixture.profile, "utf8")).toBe(original);
    } finally { await rm(fixture.dir, { recursive: true, force: true }); }
  });
  for (const [name, markers] of [
    ["missing closing marker", "# >>> shine local proxy groups >>>\nOld = select, DIRECT\n"],
    ["missing opening marker", "# <<< shine local proxy groups <<<\n"],
    ["nested markers", "# >>> shine local proxy groups >>>\n# >>> shine local proxy groups >>>\n# <<< shine local proxy groups <<<\n"],
  ]) {
    test(`build and unbuild preserve user content with ${name}`, async () => {
      const original = "[Proxy]\n#!include proxies.conf\n[Proxy Group]\n#!include groups.conf\n"
        + markers + "[Rule]\n#!include rules.conf\nFINAL,Proxy\n";
      const fixture = await makeFixture(original);
      try {
        await chmod(fixture.profile, 0o600);
        for (const script of [buildScript, unbuildScript] as const) {
          await expect(runArtifact(script, fixture.profile)).rejects.toThrow("legacy Shine proxy-group block");
          expect(await readFile(fixture.profile, "utf8")).toBe(original);
          expect((await stat(fixture.profile)).mode & 0o777).toBe(0o600);
        }
      } finally {
        await rm(fixture.dir, { recursive: true, force: true });
      }
    });
  }

  test("build patches precedence idempotently while preserving CRLF and mode", async () => {
    const fixture = await makeFixture(
      [
        "[Proxy]",
        "#!include provider-proxies.conf",
        "[Proxy Group]",
        "# >>> shine local proxy groups >>>",
        "Old Inline = select, DIRECT",
        "# <<< shine local proxy groups <<<",
        "#!include provider-groups.conf",
        "[Rule]",
        "#!include provider-rules.conf",
        "",
      ].join("\r\n"),
    );
    try {
      await chmod(fixture.profile, 0o640);
      const first = await runArtifact(buildScript, fixture.profile);
      expect(first.changed).toBe(true);

      const patched = await readFile(fixture.profile, "utf8");
      expect(patched).toContain(
        "#!include provider-proxies.conf, local-proxies.conf\r\n",
      );
      expect(patched).toContain(
        "#!include provider-groups.conf, local-proxy-groups.conf\r\n",
      );
      expect(patched).toContain(
        "#!include local-rules.conf, provider-rules.conf\r\n",
      );
      expect(patched).not.toContain("shine local proxy groups");
      expect(patched.replace(/\r\n/g, "")).not.toContain("\n");
      expect((await stat(fixture.profile)).mode & 0o777).toBe(0o640);

      const second = await runArtifact(buildScript, fixture.profile);
      expect(second.changed).toBe(false);
      expect(await readFile(fixture.profile, "utf8")).toBe(patched);
    } finally {
      await rm(fixture.dir, { recursive: true, force: true });
    }
  });

  test("build fails without a patchable include and leaves the profile unchanged", async () => {
    const fixture = await makeFixture(
      "[Proxy]\nDirect = direct\n[Proxy Group]\n#!include groups.conf\n[Rule]\n#!include rules.conf\n",
    );
    try {
      const before = await readFile(fixture.profile, "utf8");
      await expect(runArtifact(buildScript, fixture.profile)).rejects.toThrow(
        "[Proxy] has no #!include directive to patch",
      );
      expect(await readFile(fixture.profile, "utf8")).toBe(before);
    } finally {
      await rm(fixture.dir, { recursive: true, force: true });
    }
  });

  test("unbuild reverses local includes and removes an empty directive", async () => {
    const fixture = await makeFixture(
      [
        "[Proxy]",
        "#!include provider-proxies.conf, local-proxies.conf",
        "[Proxy Group]",
        "#!include local-proxy-groups.conf",
        "[Rule]",
        "#!include local-rules.conf, provider-rules.conf",
        "",
      ].join("\n"),
    );
    try {
      const result = await runArtifact(unbuildScript, fixture.profile);
      expect(result.changed).toBe(true);
      expect(await readFile(fixture.profile, "utf8")).toBe(
        [
          "[Proxy]",
          "#!include provider-proxies.conf",
          "[Proxy Group]",
          "[Rule]",
          "#!include provider-rules.conf",
          "",
        ].join("\n"),
      );

      const second = await runArtifact(unbuildScript, fixture.profile);
      expect(second.changed).toBe(false);
    } finally {
      await rm(fixture.dir, { recursive: true, force: true });
    }
  });

  test("build refuses to replace a symbolic-link profile", async () => {
    const fixture = await makeFixture(
      "[Proxy]\n#!include proxies.conf\n[Proxy Group]\n#!include groups.conf\n[Rule]\n#!include rules.conf\n",
    );
    const link = join(fixture.dir, "Linked.conf");
    try {
      await symlink(fixture.profile, link);
      await expect(runArtifact(buildScript, link)).rejects.toThrow("must not be a symbolic link");
    } finally {
      await rm(fixture.dir, { recursive: true, force: true });
    }
  });
});
