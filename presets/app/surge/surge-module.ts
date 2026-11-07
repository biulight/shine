import { lstat, readFile } from "node:fs/promises";

export const moduleName = "shine-general";
export const moduleFile = "shine-general.sgmodule";
export type SurgeCommand = (args: readonly string[]) => Promise<string>;

export const runSurgeCommand: SurgeCommand = async (args) => {
  const cli = "/Applications/Surge.app/Contents/Applications/surge-cli";
  try {
    const child = Bun.spawn([cli, ...args], {
      stdin: "ignore", stdout: "pipe", stderr: "pipe",
      signal: AbortSignal.timeout(10_000),
    });
    const [code, stdout] = await Promise.all([
      child.exited, new Response(child.stdout).text(), new Response(child.stderr).text(),
    ]);
    if (code !== 0) throw new Error("nonzero exit");
    return stdout;
  } catch {
    // Do not print arbitrary controller output (module names/diagnostics can be private).
    throw new Error(`surge-cli ${args.join(" ")} failed; ensure Surge is running and supports module commands, then retry`);
  }
};

export function generalModuleState(output: string): boolean | undefined {
  const lines = output.trim().split(/\r?\n/);
  if (lines.shift()?.trim() !== "Modules:") throw new Error("Unrecognized Surge module inventory");
  let state: boolean | undefined;
  for (const line of lines) {
    if (!line.trim()) continue;
    const entry = /^\s*\[(on|off)\]\s+(.+?)\s*$/.exec(line);
    if (!entry) throw new Error("Unrecognized Surge module inventory entry");
    if (entry[2] !== moduleName) continue;
    if (state !== undefined) throw new Error(`Duplicate Surge module identity: ${moduleName}`);
    state = entry[1] === "on";
  }
  return state;
}

export async function validateGeneralModule(path: string, reviewed: string | Uint8Array): Promise<void> {
  const metadata = await lstat(path).catch((error) => {
    if (error.code === "ENOENT") throw new Error(`Missing ${moduleFile}; run: shine app install surge`);
    throw error;
  });
  if (!metadata.isFile() || metadata.isSymbolicLink()) throw new Error(`${moduleFile} must be a regular file, not a symbolic link`);
  const installed = await readFile(path);
  if (!installed.equals(Buffer.from(reviewed))) {
    throw new Error(`Installed ${moduleFile} differs from the reviewed preset; review local edits and install or upgrade surge before applying`);
  }
  const text = new TextDecoder("utf-8", { fatal: true }).decode(installed);
  const names = [...text.matchAll(/^#!name=(.*)$/gm)];
  if (names.length !== 1 || names[0]![1]!.trim() !== moduleName) {
    throw new Error(`${moduleFile} must keep exactly one #!name=${moduleName} declaration`);
  }
}

export async function setGeneralModule(enabled: boolean, command: SurgeCommand): Promise<void> {
  const state = generalModuleState(await command(["module", "list"]));
  if (state === undefined) {
    if (enabled) throw new Error(`Surge has not discovered ${moduleFile}; install it in Surge's Profiles directory before applying`);
    // Native disable rejects missing modules. The caller must still reload to clear
    // a deleted file's cached effect; an absent file cannot clear remembered enable state.
    console.warn(`surge: ${moduleName} is not listed; remembered enable state can only be cleared after restoring its module file`);
    return;
  }
  if (state !== enabled) await command(["module", enabled ? "enable" : "disable", moduleName]);
  if (generalModuleState(await command(["module", "list"])) !== enabled) {
    throw new Error(`Surge module ${moduleName} did not become ${enabled ? "enabled" : "disabled"}; retry after checking Surge`);
  }
  console.log(`surge: module ${moduleName} is ${enabled ? "enabled" : "disabled"}`);
}
