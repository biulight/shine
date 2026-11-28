import { describe, expect, onTestFinished, test } from "bun:test";
import { mkdtemp, readFile, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { runProfileArtifact } from "./profile-artifact";
import { generalModuleState, moduleFile, moduleName, validateGeneralModule } from "./surge-module";

const original = "[General]\r\nskip-proxy = localhost\r\n[Proxy]\r\n#!include proxies.conf\r\n[Proxy Group]\r\n#!include groups.conf\r\n[Rule]\r\n#!include rules.conf\r\n";
const moduleList = "--raw module list";
const inventoryShapes = ["without modules", "with modules"] as const;

async function fixture(shape: typeof inventoryShapes[number] = "with modules") {
  const dir = await mkdtemp(join(tmpdir(), "shine-surge-module-"));
  onTestFinished(() => rm(dir, { recursive: true, force: true }));
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
    if (operation === moduleList) {
      const modules = [{ name: `${moduleName}-other`, enabled: true }];
      if (enabled !== undefined) modules.push({ name: moduleName, enabled });
      return JSON.stringify({
        available: modules.map(({ name }) => name),
        enabled: modules.filter((module) => module.enabled).map(({ name }) => name),
        ...(shape === "with modules" ? { modules } : {}),
      });
    }
    if (args[0] === "module" && !ignoreMutation) enabled = args[1] === "enable";
    if (operation === failAfter) throw new Error("interrupted after native state change");
    return "OK";
  };
  const run = (action: "build" | "unbuild") => runProfileArtifact(action, { profile, installedModule, command });
  return { profile, installedModule, calls, run, command,
    enabled: () => enabled,
    setEnabled: (value: boolean | undefined) => { enabled = value; },
    failAt: (value: string) => { fail = value; },
    failAfter: (value: string) => { failAfter = value; },
    ignoreMutation: () => { ignoreMutation = true; },
  };
}

describe("Surge native General module", () => {
  test("raw inventory matches the exact name in summary and detailed responses", () => {
    const other = "shine-general-other";
    expect(generalModuleState(JSON.stringify({ available: [other], enabled: [other] }))).toBeUndefined();
    expect(generalModuleState(JSON.stringify({ available: [other, moduleName], enabled: [other] }))).toBe(false);
    expect(generalModuleState(JSON.stringify({
      available: [other, moduleName],
      enabled: [moduleName],
      modules: [
        { name: other, enabled: false, type: "installed" },
        { name: moduleName, enabled: true, futureMetadata: { labels: ["extra"] } },
      ],
      futureField: true,
    }, null, 2))).toBe(true);
  });

  test("raw inventory refuses duplicate target identities even when summaries are unique", () => {
    const enabled = { name: moduleName, enabled: true };
    for (const inventory of [
      { available: [moduleName, moduleName], enabled: [] },
      { available: [moduleName], enabled: [moduleName, moduleName] },
      { available: [moduleName], enabled: [moduleName], modules: [enabled, { ...enabled, enabled: false }] },
    ]) {
      expect(() => generalModuleState(JSON.stringify(inventory))).toThrow(`Duplicate Surge module identity: ${moduleName}`);
    }
  });

  test("raw inventory refuses malformed or inconsistent state", () => {
    for (const output of [
      "Modules:\n  [on] shine-general\n",
      "null",
      "{}",
      '{"available":[1],"enabled":[]}',
      '{"available":[],"enabled":[],"modules":null}',
      '{"available":[],"enabled":[],"modules":[{"name":"shine-general","enabled":"true"}]}',
    ]) {
      expect(() => generalModuleState(output)).toThrow("Unrecognized Surge module inventory JSON");
    }
    for (const inventory of [
      { available: [], enabled: [moduleName] },
      { available: [moduleName], enabled: [], modules: [{ name: moduleName, enabled: true }] },
      { available: [moduleName], enabled: [], modules: [] },
    ]) {
      expect(() => generalModuleState(JSON.stringify(inventory))).toThrow("Inconsistent Surge module inventory");
    }
  });

  for (const shape of inventoryShapes) {
    test(`apply and repeatable removal verify native state using raw inventory ${shape}`, async () => {
      const f = await fixture(shape);
      await f.run("build");
      expect(f.enabled()).toBe(true);
      expect(await readFile(f.profile, "utf8")).toContain("#!include local-rules.conf, rules.conf");
      expect(f.calls).toEqual([moduleList, moduleList, "module enable shine-general", moduleList, "reload"]);

      f.calls.length = 0;
      await f.run("unbuild");
      expect(f.enabled()).toBe(false);
      expect(await readFile(f.profile, "utf8")).toBe(original);
      expect(f.calls).toEqual([moduleList, "module disable shine-general", moduleList, "reload", "reload"]);

      f.calls.length = 0;
      await f.run("unbuild");
      expect(f.calls).not.toContain(`module disable ${moduleName}`);
      expect(await readFile(f.profile, "utf8")).toBe(original);
      expect(await readFile(f.installedModule, "utf8")).toContain("#!name=shine-general");
    });
  }

  test("apply preserves General, patches includes, enables only its module and repeats safely", async () => {
    const f = await fixture();
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
  });

  test("remove disables before touching includes, even when profile validation fails", async () => {
    const f = await fixture();
    await f.run("build");
    await writeFile(f.profile, "[Proxy Group]\n# >>> shine local proxy groups >>>\n");
    f.calls.length = 0;
    await expect(f.run("unbuild")).rejects.toThrow("legacy Shine proxy-group block");
    expect(f.enabled()).toBe(false);
    expect(f.calls).toContain(`module disable ${moduleName}`);
    expect(f.calls).toContain("reload");
    expect(await readFile(f.profile, "utf8")).toContain("# >>> shine");
  });

  for (const problem of ["missing", "modified", "symlink", "undiscovered", "offline", "invalid-profile"]) {
    test(`apply preflight leaves Profile and native state unchanged: ${problem}`, async () => {
      const f = await fixture();
      if (problem === "missing") await rm(f.installedModule);
      if (problem === "modified") await writeFile(f.installedModule, "#!name=someone-else\n");
      if (problem === "symlink") { await rm(f.installedModule); await symlink(join(import.meta.dir, moduleFile), f.installedModule); }
      if (problem === "undiscovered") f.setEnabled(undefined);
      if (problem === "offline") f.failAt(moduleList);
      if (problem === "invalid-profile") await writeFile(f.profile, "[Rule]\nFINAL,DIRECT\n");
      const before = await readFile(f.profile, "utf8");
      await expect(f.run("build")).rejects.toThrow();
      expect(await readFile(f.profile, "utf8")).toBe(before);
      expect(f.calls.some((call) => call.startsWith("module enable"))).toBe(false);
    });
  }

  test("installed module must retain the fixed identity and match reviewed snapshot bytes", async () => {
    const f = await fixture();
    const custom = "#!name=shine-general\n[General]\nskip-proxy = %APPEND% *.internal\n";
    await writeFile(f.installedModule, custom);
    await validateGeneralModule(f.installedModule, custom);
    await expect(validateGeneralModule(f.installedModule, custom + "# changed\n")).rejects.toThrow("differs");
    const renamed = custom.replace("shine-general", "renamed");
    await writeFile(f.installedModule, renamed);
    await expect(validateGeneralModule(f.installedModule, renamed)).rejects.toThrow("#!name=shine-general");
  });

  for (const failedCommand of [`module enable ${moduleName}`, "reload"]) {
    test(`partial apply is reported and can be retried after ${failedCommand} failure`, async () => {
      const f = await fixture();
      f.failAt(failedCommand);
      await expect(f.run("build")).rejects.toThrow();
      const applied = await readFile(f.profile, "utf8");
      expect(applied).toContain("local-rules.conf");
      f.failAt("");
      await f.run("build");
      expect(f.enabled()).toBe(true);
      expect(await readFile(f.profile, "utf8")).toBe(applied);
    });
  }

  for (const shape of inventoryShapes) {
    for (const action of ["build", "unbuild"] as const) {
      test(`an acknowledged but ineffective ${action} fails verification for inventory ${shape}`, async () => {
        const f = await fixture(shape);
        f.setEnabled(action === "unbuild");
        f.ignoreMutation();
        await expect(f.run(action)).rejects.toThrow(`did not become ${action === "build" ? "enabled" : "disabled"}`);
        expect(f.calls).not.toContain("reload");
      });
    }
  }

  for (const action of ["build", "unbuild"] as const) {
    test(`retry observes native state after interrupted ${action}`, async () => {
      const f = await fixture();
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
    });
  }

  test("remove clears its module even when SURGE_PROFILE is unset", async () => {
    const f = await fixture();
    f.setEnabled(true);
    await expect(runProfileArtifact("unbuild", { profile: "", command: f.command })).rejects.toThrow("SURGE_PROFILE is not set");
    expect(f.enabled()).toBe(false);
    expect(f.calls).toContain("reload");
  });

  test.each([
    [`module disable ${moduleName}`, true],
    ["reload", false],
  ] as const)("failed %s during removal preserves includes for retry", async (operation, stillEnabled) => {
    const f = await fixture();
    await f.run("build");
    const applied = await readFile(f.profile, "utf8");
    f.failAt(operation);
    await expect(f.run("unbuild")).rejects.toThrow();
    expect(f.enabled()).toBe(stillEnabled);
    expect(await readFile(f.profile, "utf8")).toBe(applied);
    f.failAt("");
    await f.run("unbuild");
    expect(f.enabled()).toBe(false);
    expect(await readFile(f.profile, "utf8")).toBe(original);
  });

  for (const target of ["profile", "module"] as const) {
    test(`apply rechecks ${target} edits made during native preflight`, async () => {
      const f = await fixture();
      let edited = false;
      const command = async (args: readonly string[]) => {
        const result = await f.command(args);
        if (!edited && args.join(" ") === moduleList) {
          const path = target === "profile" ? f.profile : f.installedModule;
          await writeFile(path, await readFile(path, "utf8") + "# concurrent edit\n");
          edited = true;
        }
        return result;
      };
      await expect(runProfileArtifact("build", { profile: f.profile, installedModule: f.installedModule, command })).rejects.toThrow();
      expect(f.enabled()).toBe(false);
      expect(await readFile(target === "profile" ? f.profile : f.installedModule, "utf8")).toContain("# concurrent edit");
    });
  }

  test("legacy or missing module teardown still reloads to clear cached effects", async () => {
    const f = await fixture();
    await rm(f.installedModule);
    f.setEnabled(undefined);
    await f.run("unbuild");
    expect(f.calls).toContain("reload");
    expect(f.calls.some((call) => call.startsWith("module disable"))).toBe(false);
  });
});
