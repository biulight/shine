import { describe, expect, test } from "bun:test";
import { join, relative } from "node:path";

const PRESETS_ROOT = import.meta.dir;
const PRODUCTION_EXTENSIONS = [".ts", ".js", ".mts", ".mjs"];

function isProductionScript(path: string): boolean {
  return (
    PRODUCTION_EXTENSIONS.some((extension) => path.endsWith(extension)) &&
    !/\.test\.(?:ts|js|mts|mjs)$/.test(path)
  );
}

function isAllowedSpecifier(specifier: string): boolean {
  return (
    specifier.startsWith("./") ||
    specifier.startsWith("../") ||
    specifier === "bun" ||
    specifier.startsWith("bun:") ||
    specifier.startsWith("node:")
  );
}

describe("embedded Bun dependency policy", () => {
  test("production preset scripts use only relative and runtime-built-in imports", async () => {
    const violations: string[] = [];
    const glob = new Bun.Glob("**/*.{ts,js,mts,mjs}");
    for await (const entry of glob.scan({ cwd: PRESETS_ROOT, onlyFiles: true })) {
      if (!isProductionScript(entry)) continue;
      const path = join(PRESETS_ROOT, entry);
      const source = (await Bun.file(path).text()).replace(/^#![^\n]*(?:\n|$)/, "\n");
      const imports = new Bun.Transpiler({ loader: "ts" }).scanImports(source);
      for (const { path: specifier } of imports) {
        if (!isAllowedSpecifier(specifier)) {
          violations.push(`${relative(PRESETS_ROOT, path)}: ${specifier}`);
        }
      }
    }
    expect(violations).toEqual([]);
  });
});
