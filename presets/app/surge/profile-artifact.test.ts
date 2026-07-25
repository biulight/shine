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

const buildScript = join(import.meta.dir, "build.ts");
const unbuildScript = join(import.meta.dir, "unbuild.ts");

async function runArtifact(
  script: string,
  profile: string,
): Promise<{ code: number; stdout: string; stderr: string }> {
  const child = Bun.spawn([process.execPath, script], {
    env: {
      ...Bun.env,
      SHINE_SURGE_SKIP_RELOAD: "1",
      SURGE_PROFILE: profile,
    },
    stdout: "pipe",
    stderr: "pipe",
  });
  const [code, stdout, stderr] = await Promise.all([
    child.exited,
    new Response(child.stdout).text(),
    new Response(child.stderr).text(),
  ]);
  return { code, stdout, stderr };
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
      expect(first.code).toBe(0);

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
      expect(second.code).toBe(0);
      expect(second.stdout).toContain("already patched");
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
      const result = await runArtifact(buildScript, fixture.profile);
      expect(result.code).not.toBe(0);
      expect(result.stderr).toContain(
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
      expect(result.code).toBe(0);
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
      expect(second.code).toBe(0);
      expect(second.stdout).toContain("already unpatched");
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
      const result = await runArtifact(buildScript, link);
      expect(result.code).not.toBe(0);
      expect(result.stderr).toContain("must not be a symbolic link");
    } finally {
      await rm(fixture.dir, { recursive: true, force: true });
    }
  });
});
