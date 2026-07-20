// Artifact teardown script for the `clash-verge` app preset (runtime = "bun").
//
// `shine app unbuild clash-verge` (and, best-effort, `shine app uninstall
// clash-verge`) runs this via `bun`. Unlike the Surge preset — where build.sh
// patches a plain-text profile that unbuild.sh can un-patch — Clash Verge Rev
// owns the Merge-profile binding in its private store, and shine never writes it.
// There is therefore nothing local to reverse: this just tells you how to detach
// the Merge profile inside CVR. It exits 0 so uninstall is never blocked.

console.log("clash-verge: shine does not manage the CVR enhancement binding, so there is");
console.log("clash-verge: nothing to reverse locally.");
console.log("clash-verge: to fully remove it, open Clash Verge Rev → Profiles → the active");
console.log("clash-verge: subscription and clear its Extend Config, Rules, Proxies, and Groups editors.");
