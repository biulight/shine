import { expect, test } from "bun:test";
import { join } from "node:path";

type Hook = { id: string; files?: string; entry?: string };
type HookConfig = { repos: { hooks: Hook[] }[] };

const ROOT = join(import.meta.dir, "..");

test("Bun checks execute with the runtime pinned in mise.toml", async () => {
  const config = Bun.TOML.parse(await Bun.file(join(ROOT, "mise.toml")).text()) as {
    tools: { bun: string };
  };
  expect(Bun.version).toBe(config.tools.bun);
});

test.each([
  {
    id: "bun-check", glob: "presets/**/*.ts",
    inputs: ["package.json", "tsconfig.json", "bun.lock", "presets/shell/agent/cc.providers.toml"],
    excluded: ["docs/manual/index.md", "cli/src/main.rs", "presets/app/surge/shine.toml"],
  },
  {
    id: "cargo-test", glob: "presets/**/*",
    inputs: ["Cargo.toml", "Cargo.lock", ".config/nextest.toml", "tests/builtin_presets.rs",
      "tests/builtin_presets/support.rs", "core/src/runtime/mod.rs"],
    excluded: ["docs/manual/index.md"],
  },
])("$id pre-commit hook covers its preset and tooling inputs", async ({ id, glob, inputs, excluded }) => {
  const config = Bun.YAML.parse(
    await Bun.file(join(ROOT, ".pre-commit-config.yaml")).text(),
  ) as HookConfig;
  const hook = config.repos.flatMap((repo) => repo.hooks).find((hook) => hook.id === id);
  if (id === "bun-check") {
    expect(hook?.entry).toBe("bash -c 'mise exec -- bun run --bun check:ts'");
  }
  expect(hook?.files).toBeDefined();
  const filter = new RegExp(hook!.files!);
  const paths = [".pre-commit-config.yaml", "mise.toml", ...inputs];
  for await (const path of new Bun.Glob(glob).scan({ cwd: ROOT, onlyFiles: true })) {
    paths.push(path.replaceAll("\\", "/"));
  }
  expect(paths.filter((path) => !filter.test(path))).toEqual([]);
  expect(excluded.filter((path) => filter.test(path))).toEqual([]);
});
