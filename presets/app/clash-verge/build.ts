// Artifact build script for the `clash-verge` app preset (runtime = "bun").
//
// `shine app build clash-verge` runs this via `bun` on macOS/Windows/Linux and:
//   1. Writes this preset's merge.yaml into Clash Verge Rev's Global Extend Config
//      file (`<CVR data dir>/profiles/Merge.yaml`) — so you never paste it by hand.
//      The analog of surge's build.sh writing the Surge profile. shine writes only
//      this user-content merge file, never CVR's profiles.yaml / binding DB / cache.
//   2. Refreshes the mihomo rule-providers via the external controller (the analog
//      of `surge-cli reload`), so edited LAN rule lists apply immediately instead
//      of waiting for their `interval`.
//
// One-time setup in CVR: create an (empty) Global Extend Config so CVR registers
// the Merge slot; then this script owns its content. The Merge.yaml path is
// auto-detected per platform; override with `shine env set CLASH_MERGE_FILE <path>`
// (a per-machine value — do NOT put it in a shared overlay's shine.env.toml).
//
// Controller env (only needed for the immediate refresh; read verbatim/plaintext):
//   shine env set CLASH_CONTROLLER_URL   http://127.0.0.1:9097
//   shine env set CLASH_CONTROLLER_TOKEN <secret from CVR → Settings → Clash Core>

import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";

const CVR_ID = "io.github.clash-verge-rev.clash-verge-rev";

function expandTilde(p: string): string {
  const home = Bun.env.HOME ?? "";
  if (p === "~") return home;
  if (p.startsWith("~/")) return `${home}/${p.slice(2)}`;
  return p;
}

// Default CVR Global Extend Config path (`Merge.yaml`) per platform.
function defaultMergePath(): string | null {
  if (process.platform === "win32") {
    const appData = Bun.env.APPDATA;
    return appData ? `${appData}\\${CVR_ID}\\profiles\\Merge.yaml` : null;
  }
  if (process.platform === "darwin") {
    const home = Bun.env.HOME;
    return home ? `${home}/Library/Application Support/${CVR_ID}/profiles/Merge.yaml` : null;
  }
  const data = Bun.env.XDG_DATA_HOME ?? (Bun.env.HOME ? `${Bun.env.HOME}/.local/share` : null);
  return data ? `${data}/${CVR_ID}/profiles/Merge.yaml` : null;
}

// Write merge.yaml into CVR's Merge.yaml, idempotently. Non-fatal: a missing
// target dir just means the user hasn't created the Global Extend Config yet.
function writeMergeFile(): void {
  const raw = Bun.env.CLASH_MERGE_FILE;
  const target = raw ? expandTilde(raw) : defaultMergePath();
  if (!target) {
    console.log("clash-verge: could not resolve CVR's Merge.yaml path — set CLASH_MERGE_FILE to enable auto-write.");
    return;
  }

  const dir = dirname(target);
  if (!existsSync(dir)) {
    console.log(`clash-verge: ${dir} not found.`);
    console.log("clash-verge: create an (empty) Global Extend Config in CVR first (or set CLASH_MERGE_FILE),");
    console.log("clash-verge: then re-run `shine app build clash-verge`. Skipping Merge.yaml write.");
    return;
  }

  // Read the RESOLVED merge.yaml: the overlay's real copy wins over the base
  // example. build.ts itself lives in base (the overlay ships no build.ts), so
  // import.meta.dir points at base — mirror shine's per-file overlay resolution
  // via SHINE_APP_OVERLAY_DIR (`<overlay>/app/clash-verge`, set only when present).
  const overlayDir = Bun.env.SHINE_APP_OVERLAY_DIR;
  const overlayMerge = overlayDir ? `${overlayDir}/merge.yaml` : "";
  const source = overlayMerge && existsSync(overlayMerge) ? overlayMerge : `${import.meta.dir}/merge.yaml`;

  const header = "# Managed by shine (app/clash-verge). Edit your overlay's merge.yaml, not this file.\n";
  const content = header + readFileSync(source, "utf8");
  if (existsSync(target) && readFileSync(target, "utf8") === content) {
    console.log(`clash-verge: Merge.yaml already up to date (${target})`);
    return;
  }
  writeFileSync(target, content);
  console.log(`clash-verge: wrote CVR Merge.yaml (${target})`);
}

writeMergeFile();

// Immediate rule-provider refresh via the mihomo external controller (optional).
const url = (Bun.env.CLASH_CONTROLLER_URL ?? "").replace(/\/+$/, "");
if (!url) {
  console.log("clash-verge: CLASH_CONTROLLER_URL not set — skipping the immediate rule-provider refresh");
  console.log("clash-verge: (rules still refresh on their interval).");
  process.exit(0);
}

const token = Bun.env.CLASH_CONTROLLER_TOKEN ?? "";
const headers: Record<string, string> = token ? { Authorization: `Bearer ${token}` } : {};

// Provider names must match the `rule-providers:` keys in merge.yaml.
const providers = ["lan", "lan-socks", "other-direct"];

let failed = false;
for (const name of providers) {
  try {
    // mihomo returns 204 on a successful provider refresh.
    const res = await fetch(`${url}/providers/rules/${name}`, { method: "PUT", headers });
    if (res.ok) {
      console.log(`clash-verge: refreshed rule-provider '${name}'`);
    } else {
      console.error(`clash-verge: failed to refresh '${name}' (HTTP ${res.status}) via ${url}`);
      failed = true;
    }
  } catch (err) {
    console.error(`clash-verge: could not reach the controller for '${name}' via ${url}: ${err}`);
    failed = true;
  }
}

if (failed) {
  console.error(
    "clash-verge: is Clash Verge Rev running, the Merge profile imported, and CLASH_CONTROLLER_URL/TOKEN correct?",
  );
  process.exit(1);
}

console.log("clash-verge: all rule-providers refreshed.");
