import { describe, expect, onTestFinished, test } from "bun:test";
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
import { dirname, join } from "node:path";
import { applyProfileArtifact } from "./profile-artifact";

async function makeProfile(profileContent: string): Promise<string> {
  const dir = await mkdtemp(join(tmpdir(), "shine-surge-artifact-"));
  onTestFinished(() => rm(dir, { recursive: true, force: true }));
  const profile = join(dir, "Profile.conf");
  await writeFile(profile, profileContent);
  for (const name of [
    "local-proxies.conf",
    "local-proxy-groups.conf",
    "local-rules.conf",
  ]) {
    await writeFile(join(dir, name), "# fixture\n");
  }
  return profile;
}

describe("Surge profile artifact", () => {
  test("include apply/remove preserves a BOM and leaves General unchanged", async () => {
    const original = "\uFEFF[General]\r\nskip-proxy = localhost\r\n[Proxy]\r\n#!include proxies.conf\r\n[Proxy Group]\r\n#!include groups.conf\r\n[Rule]\r\n#!include rules.conf\r\n";
    const profile = await makeProfile(original);
    await applyProfileArtifact("build", profile);
    const applied = await readFile(profile, "utf8");
    expect(applied.startsWith("\uFEFF[General]\r\nskip-proxy = localhost\r\n")).toBe(true);
    expect((await applyProfileArtifact("build", profile)).changed).toBe(false);
    await applyProfileArtifact("unbuild", profile);
    expect(await readFile(profile, "utf8")).toBe(original);
  });
  for (const [name, markers] of [
    ["missing closing marker", "# >>> shine local proxy groups >>>\nOld = select, DIRECT\n"],
    ["missing opening marker", "# <<< shine local proxy groups <<<\n"],
    ["nested markers", "# >>> shine local proxy groups >>>\n# >>> shine local proxy groups >>>\n# <<< shine local proxy groups <<<\n"],
  ]) {
    test(`build and unbuild preserve user content with ${name}`, async () => {
      const original = "[Proxy]\n#!include proxies.conf\n[Proxy Group]\n#!include groups.conf\n"
        + markers + "[Rule]\n#!include rules.conf\nFINAL,Proxy\n";
      const profile = await makeProfile(original);
      await chmod(profile, 0o600);
      // Windows exposes limited mode bits; preserve the observed mode on every OS.
      const originalMode = (await stat(profile)).mode & 0o777;
      if (process.platform !== "win32") expect(originalMode).toBe(0o600);
      for (const script of ["build", "unbuild"] as const) {
        await expect(applyProfileArtifact(script, profile)).rejects.toThrow("legacy Shine proxy-group block");
        expect(await readFile(profile, "utf8")).toBe(original);
        expect((await stat(profile)).mode & 0o777).toBe(originalMode);
      }
    });
  }

  test("build patches precedence idempotently while preserving CRLF and mode", async () => {
    const profile = await makeProfile(
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
    await chmod(profile, 0o640);
    const originalMode = (await stat(profile)).mode & 0o777;
    if (process.platform !== "win32") expect(originalMode).toBe(0o640);
    const first = await applyProfileArtifact("build", profile);
    expect(first.changed).toBe(true);

    const patched = await readFile(profile, "utf8");
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
    expect((await stat(profile)).mode & 0o777).toBe(originalMode);

    const second = await applyProfileArtifact("build", profile);
    expect(second.changed).toBe(false);
    expect(await readFile(profile, "utf8")).toBe(patched);
  });

  test("build fails without a patchable include and leaves the profile unchanged", async () => {
    const profile = await makeProfile(
      "[Proxy]\nDirect = direct\n[Proxy Group]\n#!include groups.conf\n[Rule]\n#!include rules.conf\n",
    );
    const before = await readFile(profile, "utf8");
    await expect(applyProfileArtifact("build", profile)).rejects.toThrow(
      "[Proxy] has no #!include directive to patch",
    );
    expect(await readFile(profile, "utf8")).toBe(before);
  });

  test("unbuild reverses local includes and removes an empty directive", async () => {
    const profile = await makeProfile(
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
    const result = await applyProfileArtifact("unbuild", profile);
    expect(result.changed).toBe(true);
    expect(await readFile(profile, "utf8")).toBe(
      [
        "[Proxy]",
        "#!include provider-proxies.conf",
        "[Proxy Group]",
        "[Rule]",
        "#!include provider-rules.conf",
        "",
      ].join("\n"),
    );

    const second = await applyProfileArtifact("unbuild", profile);
    expect(second.changed).toBe(false);
  });

  test("build refuses to replace a symbolic-link profile", async () => {
    const profile = await makeProfile(
      "[Proxy]\n#!include proxies.conf\n[Proxy Group]\n#!include groups.conf\n[Rule]\n#!include rules.conf\n",
    );
    const link = join(dirname(profile), "Linked.conf");
    await symlink(profile, link);
    await expect(applyProfileArtifact("build", link)).rejects.toThrow("must not be a symbolic link");
  });
});
