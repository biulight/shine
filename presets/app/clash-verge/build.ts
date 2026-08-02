// Artifact build script for the `clash-verge` app preset (runtime = "bun").
//
// CVR 2.x stores subscription enhancements in four separate bound files:
// merge (ordinary mapping keys), rules, proxies, and groups. The latter three
// use { prepend, append, delete } documents. This script treats merge.yaml as a
// composite shine-owned source and renders it into those CVR-native files.

import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { basename, dirname, join } from "node:path";

const CVR_ID = "io.github.clash-verge-rev.clash-verge-rev";
const PROVIDERS = ["lan", "lan-socks", "other-direct"];
const BINDING_KINDS = ["merge", "rules", "proxies", "groups"] as const;

type BindingKind = (typeof BINDING_KINDS)[number];
type ProfileItem = {
  uid?: unknown;
  type?: unknown;
  file?: unknown;
  option?: Partial<Record<BindingKind, unknown>> | unknown;
};
type ProfilesConfig = { current?: unknown; items?: unknown };
export type BoundFiles = Record<BindingKind, string>;
export type RenderedPayload = Record<BindingKind, string>;

function expandTilde(path: string): string {
  const home = Bun.env.HOME ?? Bun.env.USERPROFILE ?? "";
  if (path === "~") return home;
  if (path.startsWith("~/") || path.startsWith("~\\")) return join(home, path.slice(2));
  return path;
}

export function defaultCvrDataDir(): string | null {
  if (process.platform === "win32") {
    return Bun.env.APPDATA ? join(Bun.env.APPDATA, CVR_ID) : null;
  }
  if (process.platform === "darwin") {
    return Bun.env.HOME ? join(Bun.env.HOME, "Library", "Application Support", CVR_ID) : null;
  }
  const data = Bun.env.XDG_DATA_HOME ?? (Bun.env.HOME ? join(Bun.env.HOME, ".local", "share") : null);
  return data ? join(data, CVR_ID) : null;
}

function profileItems(config: ProfilesConfig): ProfileItem[] {
  return Array.isArray(config.items)
    ? config.items.filter((item): item is ProfileItem => typeof item === "object" && item !== null)
    : [];
}

export function resolveBoundFiles(profilesPath: string): BoundFiles | null {
  if (!existsSync(profilesPath)) return null;

  const parsed = Bun.YAML.parse(readFileSync(profilesPath, "utf8")) as ProfilesConfig | null;
  if (!parsed || typeof parsed !== "object" || typeof parsed.current !== "string") return null;

  const items = profileItems(parsed);
  const current = items.find((item) => item.uid === parsed.current);
  const option = current?.option;
  if (!option || typeof option !== "object") return null;
  const bindings = option as Partial<Record<BindingKind, unknown>>;

  const profilesDir = join(dirname(profilesPath), "profiles");
  const result = {} as BoundFiles;
  for (const kind of BINDING_KINDS) {
    const uid = bindings[kind];
    if (typeof uid !== "string") return null;
    const item = items.find((candidate) => candidate.uid === uid && candidate.type === kind);
    if (!item || typeof item.file !== "string") return null;
    if (item.file === "." || item.file === ".." || basename(item.file) !== item.file) return null;
    result[kind] = join(profilesDir, item.file);
  }
  return result;
}

function resolveProfilesPath(): string | null {
  const override = Bun.env.CLASH_PROFILES_FILE?.trim();
  if (override) return expandTilde(override);
  const dataDir = defaultCvrDataDir();
  return dataDir ? join(dataDir, "profiles.yaml") : null;
}

function resolvedPayloadSource(): string {
  const overlayDir = Bun.env.SHINE_APP_OVERLAY_DIR;
  const overlayMerge = overlayDir ? join(overlayDir, "merge.yaml") : "";
  return overlayMerge && existsSync(overlayMerge) ? overlayMerge : join(import.meta.dir, "merge.yaml");
}

function takeArray(mapping: Record<string, unknown>, key: string): unknown[] {
  const value = mapping[key];
  delete mapping[key];
  if (value === undefined) return [];
  if (!Array.isArray(value)) throw new Error(`clash-verge: '${key}' must be a YAML array`);
  return value;
}

function renderEditorFile(values: unknown[]): string {
  return Bun.YAML.stringify({ prepend: values, append: [], delete: [] });
}

export function renderPayload(source: string): RenderedPayload {
  const parsed = Bun.YAML.parse(readFileSync(source, "utf8"));
  if (parsed !== null && (typeof parsed !== "object" || Array.isArray(parsed))) {
    throw new Error("clash-verge: merge.yaml must contain a YAML mapping");
  }
  const merge = { ...((parsed ?? {}) as Record<string, unknown>) };
  const proxies = takeArray(merge, "proxies");
  const groups = takeArray(merge, "proxy-groups");
  const rules = takeArray(merge, "prepend-rules");

  return {
    merge: Bun.YAML.stringify(merge),
    rules: renderEditorFile(rules),
    proxies: renderEditorFile(proxies),
    groups: renderEditorFile(groups),
  };
}

function managedContent(kind: BindingKind, content: string): string {
  return `# Managed by shine (app/clash-verge, ${kind}). Edit your overlay's merge.yaml, not this file.\n${content}`;
}

function canonicalYaml(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(canonicalYaml);
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value as Record<string, unknown>)
        .sort(([left], [right]) => left.localeCompare(right))
        .map(([key, child]) => [key, canonicalYaml(child)]),
    );
  }
  return value;
}

function yamlEquivalent(left: string, right: string): boolean {
  try {
    return JSON.stringify(canonicalYaml(Bun.YAML.parse(left))) === JSON.stringify(canonicalYaml(Bun.YAML.parse(right)));
  } catch {
    return false;
  }
}

export function installPayload(payload: RenderedPayload, targets: BoundFiles): "changed" | "current" {
  let changed = false;
  for (const kind of BINDING_KINDS) {
    const content = managedContent(kind, payload[kind]);
    if (!existsSync(targets[kind]) || !yamlEquivalent(readFileSync(targets[kind], "utf8"), content)) {
      writeFileSync(targets[kind], content);
      changed = true;
    }
  }
  return changed ? "changed" : "current";
}

async function refreshProviders(): Promise<boolean> {
  const url = (Bun.env.CLASH_CONTROLLER_URL ?? "").replace(/\/+$/, "");
  if (!url) {
    console.log("clash-verge: CLASH_CONTROLLER_URL not set — skipping the immediate rule-provider refresh");
    console.log("clash-verge: (rules still refresh on their interval).");
    return true;
  }

  const token = Bun.env.CLASH_CONTROLLER_TOKEN ?? "";
  const headers: Record<string, string> = token ? { Authorization: `Bearer ${token}` } : {};
  let succeeded = true;
  for (const name of PROVIDERS) {
    try {
      const response = await fetch(`${url}/providers/rules/${name}`, { method: "PUT", headers });
      if (response.ok) {
        console.log(`clash-verge: refreshed rule-provider '${name}'`);
      } else {
        console.error(`clash-verge: failed to refresh '${name}' (HTTP ${response.status}) via ${url}`);
        succeeded = false;
      }
    } catch (error) {
      console.error(`clash-verge: could not reach the controller for '${name}' via ${url}: ${error}`);
      succeeded = false;
    }
  }
  return succeeded;
}

async function main(): Promise<void> {
  const profilesPath = resolveProfilesPath();
  const targets = profilesPath ? resolveBoundFiles(profilesPath) : null;
  if (!targets) {
    console.log("clash-verge: the active subscription does not have all enhancement editors bound.");
    console.log("clash-verge: open its Extend Config, Edit Rules, Edit Proxies, and Edit Groups once,");
    console.log("clash-verge: then re-run `shine app artifact apply clash-verge`.");
    return;
  }

  if (Object.values(targets).some((target) => !existsSync(dirname(target)))) {
    console.log("clash-verge: CVR's profiles directory was not found; skipping enhancement writes.");
    return;
  }

  const state = installPayload(renderPayload(resolvedPayloadSource()), targets);
  if (state === "changed") {
    console.log("clash-verge: wrote the active subscription's Merge, Rules, Proxies, and Groups enhancements");
    console.log("clash-verge: reselect the subscription in CVR once to apply the changed files;");
    console.log("clash-verge: then re-run `shine app artifact apply clash-verge` to refresh its rule-providers.");
    return;
  }
  console.log("clash-verge: active subscription enhancements already up to date");

  if (!(await refreshProviders())) {
    console.error(
      "clash-verge: is Clash Verge Rev running, the subscription enhancements applied, and CLASH_CONTROLLER_URL/TOKEN correct?",
    );
    process.exitCode = 1;
  } else {
    console.log("clash-verge: all rule-providers refreshed.");
  }
}

if (import.meta.main) await main();
