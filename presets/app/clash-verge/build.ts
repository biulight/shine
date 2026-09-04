// Artifact build script for the `clash-verge` app preset (runtime = "bun").
//
// CVR 2.x stores subscription enhancements in four separate bound files:
// merge (ordinary mapping keys), rules, proxies, and groups. The latter three
// use { prepend, append, delete } documents. This script treats merge.yaml as a
// composite shine-owned source and renders it into those CVR-native files.

import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { basename, dirname, join } from "node:path";

const CVR_ID = "io.github.clash-verge-rev.clash-verge-rev";
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
export type RenderedSource = { payload: RenderedPayload; providers: string[] };
export type RefreshResult = "refreshed" | "skipped" | "failed";
export type ConnectionCloseResult = "closed" | "failed";
export type SyncResult = "bindings-updated" | RefreshResult;
type FetchLike = (input: string, init: RequestInit) => Promise<Response>;

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

export function renderPayload(source: string): RenderedSource {
  const parsed = Bun.YAML.parse(readFileSync(source, "utf8"));
  if (parsed !== null && (typeof parsed !== "object" || Array.isArray(parsed))) {
    throw new Error("clash-verge: merge.yaml must contain a YAML mapping");
  }
  const merge = { ...((parsed ?? {}) as Record<string, unknown>) };
  const ruleProviders = merge["rule-providers"];
  if (
    ruleProviders !== undefined &&
    ruleProviders !== null &&
    (typeof ruleProviders !== "object" || Array.isArray(ruleProviders))
  ) {
    throw new Error("clash-verge: 'rule-providers' must be a YAML mapping");
  }
  const providers = ruleProviders ? Object.keys(ruleProviders as Record<string, unknown>) : [];
  const proxies = takeArray(merge, "proxies");
  const groups = takeArray(merge, "proxy-groups");
  const rules = takeArray(merge, "prepend-rules");

  return {
    payload: {
      merge: Bun.YAML.stringify(merge),
      rules: renderEditorFile(rules),
      proxies: renderEditorFile(proxies),
      groups: renderEditorFile(groups),
    },
    providers,
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

export function providerRefreshUrl(controllerUrl: string, name: string): string {
  return `${controllerUrl}/providers/rules/${encodeURIComponent(name)}`;
}

export async function closeConnections(
  controllerUrl: string,
  token: string,
  fetcher: FetchLike = fetch,
): Promise<ConnectionCloseResult> {
  const headers: Record<string, string> = token ? { Authorization: `Bearer ${token}` } : {};
  try {
    const response = await fetcher(`${controllerUrl}/connections`, { method: "DELETE", headers });
    if (response.ok) {
      console.log("clash-verge: closed active mihomo connections so applications use the refreshed rules");
      return "closed";
    }
    console.error(`clash-verge: failed to close active connections (HTTP ${response.status}) via ${controllerUrl}`);
  } catch (error) {
    console.error(`clash-verge: could not close active connections via ${controllerUrl}: ${error}`);
  }
  return "failed";
}

export async function refreshProviders(
  providers: string[],
  fetcher: FetchLike = fetch,
  controllerUrl: string = Bun.env.CLASH_CONTROLLER_URL ?? "",
  token: string = Bun.env.CLASH_CONTROLLER_TOKEN ?? "",
): Promise<RefreshResult> {
  if (providers.length === 0) {
    console.log("clash-verge: merge.yaml defines no rule-providers — skipping the immediate refresh");
    return "skipped";
  }

  const url = controllerUrl.replace(/\/+$/, "");
  if (!url) {
    console.log("clash-verge: CLASH_CONTROLLER_URL not set — skipping the immediate rule-provider refresh");
    console.log("clash-verge: (rules still refresh on their interval).");
    return "skipped";
  }

  const headers: Record<string, string> = token ? { Authorization: `Bearer ${token}` } : {};
  let succeeded = true;
  for (const name of providers) {
    try {
      const response = await fetcher(providerRefreshUrl(url, name), { method: "PUT", headers });
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
  return succeeded ? "refreshed" : "failed";
}

export async function syncPayload(
  payload: RenderedPayload,
  providers: string[],
  targets: BoundFiles,
  fetcher: FetchLike = fetch,
  controllerUrl: string = Bun.env.CLASH_CONTROLLER_URL ?? "",
  token: string = Bun.env.CLASH_CONTROLLER_TOKEN ?? "",
): Promise<SyncResult> {
  const state = installPayload(payload, targets);
  if (state === "changed") return "bindings-updated";

  const refresh = await refreshProviders(providers, fetcher, controllerUrl, token);
  if (refresh !== "refreshed") return refresh;
  const closed = await closeConnections(controllerUrl.replace(/\/+$/, ""), token, fetcher);
  return closed === "closed" ? "refreshed" : "failed";
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

  const { payload, providers } = renderPayload(resolvedPayloadSource());
  const result = await syncPayload(payload, providers, targets);
  if (result === "bindings-updated") {
    console.log("clash-verge: wrote the active subscription's Merge, Rules, Proxies, and Groups enhancements");
    console.log("clash-verge: reselect the subscription in CVR once to apply the changed files;");
    console.log("clash-verge: then re-run `shine app artifact apply clash-verge` to refresh its rule-providers.");
    return;
  }
  console.log("clash-verge: active subscription enhancements already up to date");
  if (result === "failed") {
    console.error(
      "clash-verge: is Clash Verge Rev running, the subscription enhancements applied, and CLASH_CONTROLLER_URL/TOKEN correct?",
    );
    process.exitCode = 1;
  } else if (result === "refreshed") {
    console.log("clash-verge: all rule-providers refreshed.");
  }
}

if (import.meta.main) await main();
