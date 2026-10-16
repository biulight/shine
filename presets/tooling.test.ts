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

test("Bun pre-commit hook covers all maintained TypeScript sources and tooling inputs", async () => {
  const config = Bun.YAML.parse(
    await Bun.file(join(ROOT, ".pre-commit-config.yaml")).text(),
  ) as HookConfig;
  const hook = config.repos.flatMap((repo) => repo.hooks).find((hook) => hook.id === "bun-check");
  expect(hook?.entry).toBe("bash -c 'mise exec -- bun run --bun check:ts'");
  expect(hook?.files).toBeDefined();
  const filter = new RegExp(hook!.files!);
  const paths = [".pre-commit-config.yaml", "mise.toml", "package.json", "tsconfig.json", "bun.lock"];
  for await (const path of new Bun.Glob("presets/**/*.ts").scan({ cwd: ROOT, onlyFiles: true })) {
    paths.push(path.replaceAll("\\", "/"));
  }
  const missed = paths.filter((path) => !filter.test(path));
  expect(missed).toEqual([]);
  for (const unrelated of ["docs/manual/index.md", "cli/src/main.rs", "presets/app/surge/shine.toml"]) {
    expect(filter.test(unrelated)).toBe(false);
  }
});
