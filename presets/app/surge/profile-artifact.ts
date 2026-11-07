import {
  chmod,
  lstat,
  readFile,
  rename,
  stat,
  unlink,
  writeFile,
} from "node:fs/promises";
import { homedir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import {
  generalModuleState, moduleFile, runSurgeCommand, setGeneralModule, validateGeneralModule,
} from "./surge-module";
import type { SurgeCommand } from "./surge-module";

type Line = {
  text: string;
  ending: string;
};

type LocalFiles = {
  proxies: boolean;
  groups: boolean;
  rules: boolean;
};

const targets = {
  proxies: { section: "[Proxy]", file: "local-proxies.conf" },
  groups: {
    section: "[Proxy Group]",
    file: "local-proxy-groups.conf",
  },
  rules: { section: "[Rule]", file: "local-rules.conf" },
} as const;

function splitLines(content: string): Line[] {
  if (content.length === 0) return [];
  const lines: Line[] = [];
  const pattern = /([^\r\n]*)(\r\n|\r|\n|$)/g;
  for (const match of content.matchAll(pattern)) {
    if (match[0] === "") break;
    lines.push({ text: match[1] ?? "", ending: match[2] ?? "" });
  }
  return lines;
}

function joinLines(lines: Line[]): string {
  return lines.map(({ text, ending }) => text + ending).join("");
}

function sectionName(line: string): string | undefined {
  line = line.replace(/^\uFEFF/, "");
  if (!line.startsWith("[")) return undefined;
  return line.trimEnd();
}

function includeParts(
  line: string,
): { prefix: string; operands: string[] } | undefined {
  const match = /^(#!include[ \t]+)(.*)$/.exec(line);
  if (!match) return undefined;
  return {
    prefix: match[1] ?? "#!include ",
    operands: (match[2] ?? "")
      .split(",")
      .map((operand) => operand.trim())
      .filter(Boolean),
  };
}

function withoutLegacyGroupBlock(lines: Line[]): Line[] {
  const result: Line[] = [];
  let inLegacyBlock = false;
  for (const line of lines) {
    if (line.text === "# >>> shine local proxy groups >>>") {
      if (inLegacyBlock) throw new Error("nested legacy Shine proxy-group block");
      inLegacyBlock = true;
      continue;
    }
    if (line.text === "# <<< shine local proxy groups <<<" && !inLegacyBlock) {
      throw new Error("legacy Shine proxy-group block has no opening marker");
    }
    if (inLegacyBlock) {
      if (line.text === "# <<< shine local proxy groups <<<") {
        inLegacyBlock = false;
      }
      continue;
    }
    result.push(line);
  }
  if (inLegacyBlock) throw new Error("legacy Shine proxy-group block has no closing marker");
  return result;
}

export function patchProfile(content: string, local: LocalFiles): string {
  const lines = withoutLegacyGroupBlock(splitLines(content));
  const completed = new Set<string>();
  let section = "";

  const patched = lines.map((line) => {
    section = sectionName(line.text) ?? section;
    const include = includeParts(line.text);
    if (!include) return line;

    const target = Object.values(targets).find(
      (candidate) =>
        candidate.section === section &&
        local[
          candidate.file === targets.proxies.file
            ? "proxies"
            : candidate.file === targets.groups.file
              ? "groups"
              : "rules"
        ],
    );
    if (!target || completed.has(target.file)) return line;

    const operands = include.operands.filter(
      (operand) => operand !== target.file,
    );
    if (target === targets.rules) operands.unshift(target.file);
    else operands.push(target.file);
    completed.add(target.file);
    return { ...line, text: include.prefix + operands.join(", ") };
  });

  const missing = Object.entries(targets)
    .filter(([key, target]) => local[key as keyof LocalFiles] && !completed.has(target.file))
    .map(([, target]) => `${target.section} has no #!include directive to patch`);
  if (missing.length > 0) throw new Error(missing.join("; "));

  return joinLines(patched);
}

export function unpatchProfile(content: string): string {
  const lines = withoutLegacyGroupBlock(splitLines(content));
  const result: Line[] = [];
  let section = "";

  for (const line of lines) {
    section = sectionName(line.text) ?? section;
    const include = includeParts(line.text);
    const target = Object.values(targets).find(
      (candidate) => candidate.section === section,
    );
    if (!include || !target) {
      result.push(line);
      continue;
    }

    const operands = include.operands.filter(
      (operand) => operand !== target.file,
    );
    if (operands.length > 0) {
      result.push({ ...line, text: include.prefix + operands.join(", ") });
    }
  }

  return joinLines(result);
}

function expandProfilePath(value: string): string {
  if (value === "~") return homedir();
  if (value.startsWith("~/")) return join(homedir(), value.slice(2));
  return resolve(value);
}

async function isRegularFile(path: string): Promise<boolean> {
  try {
    return (await stat(path)).isFile();
  } catch {
    return false;
  }
}

async function readUtf8(path: string): Promise<string> {
  const bytes = await readFile(path);
  try {
    return new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(bytes);
  } catch {
    throw new Error(`Surge profile is not valid UTF-8: ${path}`);
  }
}

async function prepareProfileArtifact(
  action: "build" | "unbuild",
  configuredPath: string,
) {
  const profile = expandProfilePath(configuredPath);
  const metadata = await lstat(profile).catch(() => undefined);
  if (!metadata) throw new Error(`SURGE_PROFILE is not a file: ${profile}`);
  if (metadata.isSymbolicLink()) {
    throw new Error(`SURGE_PROFILE must not be a symbolic link: ${profile}`);
  }
  if (!metadata.isFile()) {
    throw new Error(`SURGE_PROFILE is not a file: ${profile}`);
  }

  const profileDir = dirname(profile);
  const original = await readUtf8(profile);
  let desired: string;
  if (action === "build") {
    const local: LocalFiles = {
      proxies: await isRegularFile(join(profileDir, targets.proxies.file)),
      groups: await isRegularFile(join(profileDir, targets.groups.file)),
      rules: await isRegularFile(join(profileDir, targets.rules.file)),
    };
    if (!local.proxies && !local.groups && !local.rules) {
      throw new Error(
        `no shine-managed local Surge files found beside ${profile}; run: shine app install surge`,
      );
    }
    desired = patchProfile(original, local);
  } else {
    desired = unpatchProfile(original);
  }

  return async (): Promise<{ changed: boolean; profile: string }> => {
    const verifyUnchanged = async () => {
      const current = await lstat(profile);
      if (!current.isFile() || current.isSymbolicLink() || current.ino !== metadata.ino
        || current.dev !== metadata.dev || current.mode !== metadata.mode || await readUtf8(profile) !== original) {
        throw new Error("Surge profile changed during preparation; review and retry");
      }
    };
    await verifyUnchanged();
    if (desired === original) return { changed: false, profile };
    const temporary = join(profileDir, `.surge-${action}.${basename(profile)}.${crypto.randomUUID()}.tmp`);
    try {
      await writeFile(temporary, desired, { flag: "wx", mode: metadata.mode });
      await chmod(temporary, metadata.mode & 0o7777);
      await verifyUnchanged();
      await rename(temporary, profile);
    } finally {
      await unlink(temporary).catch(() => undefined);
    }
    return { changed: true, profile };
  };
}

export async function applyProfileArtifact(action: "build" | "unbuild", configuredPath: string) {
  return (await prepareProfileArtifact(action, configuredPath))();
}

export async function runProfileArtifact(
  action: "build" | "unbuild",
  options: { profile?: string; installedModule?: string; command?: SurgeCommand } = {},
): Promise<void> {
  const command = options.command ?? runSurgeCommand;
  // Uninstall calls teardown before file removal. Disable first even if the
  // profile is missing/malformed, so include cleanup cannot strand an enabled module.
  if (action === "unbuild") {
    await setGeneralModule(false, command);
    await command(["reload"]);
  }
  const configuredPath = options.profile ?? Bun.env.SURGE_PROFILE;
  if (!configuredPath) {
    throw new Error(
      "SURGE_PROFILE is not set — run: shine env set SURGE_PROFILE /absolute/path/to/Profile.conf",
    );
  }
  const apply = await prepareProfileArtifact(action, configuredPath);
  let installedModule: string | undefined;
  let reviewedModule: Buffer | undefined;
  if (action === "build") {
    installedModule = options.installedModule ?? join(homedir(), "Library/Application Support/Surge/Profiles", moduleFile);
    reviewedModule = await readFile(join(import.meta.dir, moduleFile));
    await validateGeneralModule(installedModule, reviewedModule);
    if (generalModuleState(await command(["module", "list"])) === undefined) {
      throw new Error(`Surge has not discovered ${moduleFile}; run: shine app install surge`);
    }
  }
  const result = await apply();
  if (action === "build") {
    await validateGeneralModule(installedModule!, reviewedModule!);
    await setGeneralModule(true, command);
    await command(["reload"]);
  } else if (result.changed) {
    await command(["reload"]);
  }
  const verb = action === "build" ? "patched" : "unpatched";
  console.log(
    result.changed
      ? `surge: ${verb} ${result.profile}`
      : `surge: profile already ${verb} — no change (${result.profile})`,
  );
}
