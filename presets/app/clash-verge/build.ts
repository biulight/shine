// Artifact build script for the `clash-verge` app preset (runtime = "bun").
//
// `shine app build clash-verge` runs this via `bun` on macOS/Windows/Linux to
// force mihomo (the core behind Clash Verge Rev) to re-pull the LAN-served rule
// lists NOW, instead of waiting for the rule-providers' `interval`. It is the
// cross-platform analog of `surge-cli reload`: after editing the shared rule
// lists and running `shine task run upload_surge`, run this to apply immediately.
//
// shine injects the active `[env]` table into the artifact verbatim, so set it
// once (values are read as stored — no decryption):
//   shine env set CLASH_CONTROLLER_URL   http://127.0.0.1:9097
//   shine env set CLASH_CONTROLLER_TOKEN <secret from CVR → Settings → Clash Core>
// CLASH_CONTROLLER_TOKEN is optional (omit for a no-auth controller). Store it in
// PLAINTEXT — not a `_SECRET`/encrypted key, which would arrive here as ciphertext.
//
// Idempotent and non-destructive: it only refreshes provider contents; it never
// writes CVR's config or its private profile store.

const url = (Bun.env.CLASH_CONTROLLER_URL ?? "").replace(/\/+$/, "");
if (!url) {
  console.error("clash-verge: run: shine env set CLASH_CONTROLLER_URL http://127.0.0.1:9097");
  process.exit(1);
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
