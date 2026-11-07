import { describe, expect, test } from "bun:test";
import { mkdtemp, readFile, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { runProfileArtifact } from "./profile-artifact";
import { generalModuleState, moduleFile, moduleName, validateGeneralModule } from "./surge-module";

const original = "[General]\r\nskip-proxy = localhost\r\n[Proxy]\r\n#!include proxies.conf\r\n[Proxy Group]\r\n#!include groups.conf\r\n[Rule]\r\n#!include rules.conf\r\n";

async function fixture() {
  const dir = await mkdtemp(join(tmpdir(), "shine-surge-module-"));
  const profile = join(dir, "Profile.conf");
  const installedModule = join(dir, moduleFile);
  await writeFile(profile, original);
  await writeFile(installedModule, await readFile(join(import.meta.dir, moduleFile)));
  for (const name of ["local-proxies.conf", "local-proxy-groups.conf", "local-rules.conf"]) {
    await writeFile(join(dir, name), "# fixture\n");
  }
  let enabled: boolean | undefined = false;
  let fail = "";
  let failAfter = "";
  let ignoreMutation = false;
  const calls: string[] = [];
  const command = async (args: readonly string[]) => {
    const operation = args.join(" ");
    calls.push(operation);
    if (operation === fail) throw new Error("simulated Surge failure");
    if (operation === "module list") return `Modules:\n  [on] unrelated-module\n${enabled === undefined ? "" : `  [${enabled ? "on" : "off"}] ${moduleName}\n`}`;
    if (args[0] === "module" && !ignoreMutation) enabled = args[1] === "enable";
    if (operation === failAfter) throw new Error("interrupted after native state change");
    return "OK";
  };
  const run = (action: "build" | "unbuild") => runProfileArtifact(action, { profile, installedModule, command });
  return { dir, profile, installedModule, calls, run, command,
    enabled: () => enabled,
    setEnabled: (value: boolean | undefined) => { enabled = value; },
    failAt: (value: string) => { fail = value; },
    failAfter: (value: string) => { failAfter = value; },
    ignoreMutation: () => { ignoreMutation = true; },
  };
}

describe("Surge native General module", () => {
  test("module inventory is exact and fails closed on malformed or duplicate output", () => {
    expect(generalModuleState("Modules:\n  [off] shine-general\n")).toBe(false);
    expect(generalModuleState("Modules:\n  [on] shine-general\n")).toBe(true);
    expect(generalModuleState("Modules:\n  [on] shine-general-other\n")).toBeUndefined();
    for (const text of ["", "Surge is not running", "Modules:\n  [unknown] shine-general\n", "Modules:\n  [on] shine-general\n  [off] shine-general\n"]) {
      expect(() => generalModuleState(text)).toThrow();
    }
  });

  test("apply preserves General, patches includes, enables only its module and repeats safely", async () => {
    const f = await fixture();
    try {
      await f.run("build");
      const applied = await readFile(f.profile, "utf8");
      expect(applied).toContain("skip-proxy = localhost\r\n");
      expect(applied).toContain("#!include local-rules.conf, rules.conf");
      expect(f.enabled()).toBe(true);
      expect(f.calls.filter((call) => call.startsWith("module enable"))).toEqual([`module enable ${moduleName}`]);
      f.calls.length = 0;
      await f.run("build");
      expect(await readFile(f.profile, "utf8")).toBe(applied);
      expect(f.calls).not.toContain(`module enable ${moduleName}`);
      expect(f.calls).toContain("reload");
    } finally { await rm(f.dir, { recursive: true, force: true }); }
  });

  test("remove disables before touching includes, even when profile validation fails", async () => {
    const f = await fixture();
    try {
      await f.run("build");
      await writeFile(f.profile, "[Proxy Group]\n# >>> shine local proxy groups >>>\n");
      f.calls.length = 0;
      await expect(f.run("unbuild")).rejects.toThrow("legacy Shine proxy-group block");
      expect(f.enabled()).toBe(false);
      expect(f.calls).toContain(`module disable ${moduleName}`);
      expect(f.calls).toContain("reload");
      expect(await readFile(f.profile, "utf8")).toContain("# >>> shine");
    } finally { await rm(f.dir, { recursive: true, force: true }); }
  });

  test("remove reverses includes and is repeatable; it does not delete the managed module file", async () => {
    const f = await fixture();
    try {
      await f.run("build");
      await f.run("unbuild");
      expect(f.enabled()).toBe(false);
      expect(await readFile(f.profile, "utf8")).toBe(original);
      f.calls.length = 0;
      await f.run("unbuild");
      expect(f.calls).not.toContain(`module disable ${moduleName}`);
      expect(await readFile(f.profile, "utf8")).toBe(original);
      expect(await readFile(f.installedModule, "utf8")).toContain("#!name=shine-general");
    } finally { await rm(f.dir, { recursive: true, force: true }); }
  });

  for (const problem of ["missing", "modified", "symlink", "undiscovered", "offline", "invalid-profile"]) {
    test(`apply preflight leaves Profile and native state unchanged: ${problem}`, async () => {
      const f = await fixture();
      try {
        if (problem === "missing") await rm(f.installedModule);
        if (problem === "modified") await writeFile(f.installedModule, "#!name=someone-else\n");
        if (problem === "symlink") { await rm(f.installedModule); await symlink(join(import.meta.dir, moduleFile), f.installedModule); }
        if (problem === "undiscovered") f.setEnabled(undefined);
        if (problem === "offline") f.failAt("module list");
        if (problem === "invalid-profile") await writeFile(f.profile, "[Rule]\nFINAL,DIRECT\n");
        const before = await readFile(f.profile, "utf8");
        await expect(f.run("build")).rejects.toThrow();
        expect(await readFile(f.profile, "utf8")).toBe(before);
        expect(f.calls.some((call) => call.startsWith("module enable"))).toBe(false);
      } finally { await rm(f.dir, { recursive: true, force: true }); }
    });
  }

  test("installed module must retain the fixed identity and match reviewed snapshot bytes", async () => {
    const f = await fixture();
    try {
      const custom = "#!name=shine-general\n[General]\nskip-proxy = %APPEND% *.internal\n";
      await writeFile(f.installedModule, custom);
      await validateGeneralModule(f.installedModule, custom);
      await expect(validateGeneralModule(f.installedModule, custom + "# changed\n")).rejects.toThrow("differs");
      const renamed = custom.replace("shine-general", "renamed");
      await writeFile(f.installedModule, renamed);
      await expect(validateGeneralModule(f.installedModule, renamed)).rejects.toThrow("#!name=shine-general");
    } finally { await rm(f.dir, { recursive: true, force: true }); }
  });

  test("a failed disable preserves includes; retry finishes cleanup", async () => {
    const f = await fixture();
    try {
      await f.run("build");
      const applied = await readFile(f.profile, "utf8");
      f.failAt(`module disable ${moduleName}`);
      await expect(f.run("unbuild")).rejects.toThrow();
      expect(await readFile(f.profile, "utf8")).toBe(applied);
      f.failAt("");
      await f.run("unbuild");
      expect(await readFile(f.profile, "utf8")).toBe(original);
    } finally { await rm(f.dir, { recursive: true, force: true }); }
  });

  for (const failedCommand of [`module enable ${moduleName}`, "reload"]) {
    test(`partial apply is reported and can be retried after ${failedCommand} failure`, async () => {
      const f = await fixture();
      try {
        f.failAt(failedCommand);
        await expect(f.run("build")).rejects.toThrow();
        const applied = await readFile(f.profile, "utf8");
        expect(applied).toContain("local-rules.conf");
        f.failAt("");
        await f.run("build");
        expect(f.enabled()).toBe(true);
        expect(await readFile(f.profile, "utf8")).toBe(applied);
      } finally { await rm(f.dir, { recursive: true, force: true }); }
    });
  }

  test("an acknowledged but ineffective state change fails verification", async () => {
    const f = await fixture();
    try {
      f.ignoreMutation();
      await expect(f.run("build")).rejects.toThrow("did not become enabled");
    } finally { await rm(f.dir, { recursive: true, force: true }); }
  });

  for (const action of ["build", "unbuild"] as const) {
    test(`retry observes native state after interrupted ${action}`, async () => {
      const f = await fixture();
      try {
        if (action === "unbuild") await f.run("build");
        f.failAfter(`module ${action === "build" ? "enable" : "disable"} ${moduleName}`);
        await expect(f.run(action)).rejects.toThrow("interrupted");
        expect(f.enabled()).toBe(action === "build");
        f.failAfter("");
        f.calls.length = 0;
        await f.run(action);
        expect(f.calls.some((call) => /module (enable|disable)/.test(call))).toBe(false);
        expect(await readFile(f.profile, "utf8")).toBe(action === "build"
          ? original.replace("#!include proxies.conf", "#!include proxies.conf, local-proxies.conf")
            .replace("#!include groups.conf", "#!include groups.conf, local-proxy-groups.conf")
            .replace("#!include rules.conf", "#!include local-rules.conf, rules.conf")
          : original);
      } finally { await rm(f.dir, { recursive: true, force: true }); }
    });
  }

  test("remove clears its module even when SURGE_PROFILE is unset", async () => {
    const f = await fixture();
    try {
      f.setEnabled(true);
      await expect(runProfileArtifact("unbuild", { profile: "", command: f.command })).rejects.toThrow("SURGE_PROFILE is not set");
      expect(f.enabled()).toBe(false);
      expect(f.calls).toContain("reload");
    } finally { await rm(f.dir, { recursive: true, force: true }); }
  });

  test("a failed removal reload preserves includes for retry", async () => {
    const f = await fixture();
    try {
      await f.run("build");
      const applied = await readFile(f.profile, "utf8");
      f.failAt("reload");
      await expect(f.run("unbuild")).rejects.toThrow();
      expect(f.enabled()).toBe(false);
      expect(await readFile(f.profile, "utf8")).toBe(applied);
      f.failAt("");
      await f.run("unbuild");
      expect(await readFile(f.profile, "utf8")).toBe(original);
    } finally { await rm(f.dir, { recursive: true, force: true }); }
  });

  for (const target of ["profile", "module"] as const) {
    test(`apply rechecks ${target} edits made during native preflight`, async () => {
      const f = await fixture();
      try {
        let edited = false;
        const command = async (args: readonly string[]) => {
          const result = await f.command(args);
          if (!edited && args.join(" ") === "module list") {
            const path = target === "profile" ? f.profile : f.installedModule;
            await writeFile(path, await readFile(path, "utf8") + "# concurrent edit\n");
            edited = true;
          }
          return result;
        };
        await expect(runProfileArtifact("build", { profile: f.profile, installedModule: f.installedModule, command })).rejects.toThrow();
        expect(f.enabled()).toBe(false);
        expect(await readFile(target === "profile" ? f.profile : f.installedModule, "utf8")).toContain("# concurrent edit");
      } finally { await rm(f.dir, { recursive: true, force: true }); }
    });
  }

  test("legacy or missing module teardown still reloads to clear cached effects", async () => {
    const f = await fixture();
    try {
      await rm(f.installedModule);
      f.setEnabled(undefined);
      await f.run("unbuild");
      expect(f.calls).toContain("reload");
      expect(f.calls.some((call) => call.startsWith("module disable"))).toBe(false);
    } finally { await rm(f.dir, { recursive: true, force: true }); }
  });
});
