import { describe, expect, spyOn, test } from "bun:test";
import { copyFileSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  buildClaudeEnvironment,
  claudeArgs,
  main,
  providerFromChoice,
  loadProviderConfig,
  parseProviderConfig,
  isSecretDecryptCommand,
  resolveCredential,
  type CommandRunner,
} from "./cc.ts";

const config = loadProviderConfig();
const { providers } = config;

const customConfig = `
defaultProvider = "second"
[[providers]]
id = "first"
label = "First"
aliases = ["One"]
enabled = true
credentialKey = "FIRST_TOKEN"
[providers.env]
CUSTOM_PROVIDER_VALUE = "first-only"
[[providers]]
id = "second"
label = "Second"
enabled = true
credentialKey = "SECOND_TOKEN"
[providers.env]
ANTHROPIC_BASE_URL = "http://127.0.0.1:9999"
CLAUDE_CODE_MAX_CONTEXT_TOKENS = "983616"
`;

describe("provider configuration", () => {
  test("custom IDs, menu order, aliases and default are independent", () => {
    const custom = parseProviderConfig(customConfig);
    expect(providerFromChoice("1", custom).id).toBe("first");
    expect(providerFromChoice("  oNe ", custom).id).toBe("first");
    expect(providerFromChoice("SECOND", custom).id).toBe("second");
    expect(providerFromChoice("  ", custom).id).toBe("second");
    const reordered = { ...custom, providers: [...custom.providers].reverse() };
    expect(providerFromChoice("1", reordered).id).toBe("second");
    expect(providerFromChoice("2", reordered).id).toBe("first");
    expect(providerFromChoice("", reordered).id).toBe("second");
    const removed = { ...custom, providers: custom.providers.slice(1) };
    expect(providerFromChoice("1", removed).id).toBe("second");
    for (const choice of ["0", "01", "-1", "1.5", "3", "99999999999999999999", "first"]) {
      expect(() => providerFromChoice(choice, removed)).toThrow("invalid provider");
    }
  });

  test.each([
    ["missing default", customConfig.replace('defaultProvider = "second"', ""), "defaultProvider"],
    ["unknown default", customConfig.replace('defaultProvider = "second"', 'defaultProvider = "missing"'), "defaultProvider"],
    ["disabled default", customConfig.replaceAll("enabled = true", "enabled = false"), "defaultProvider"],
    ["empty list", 'defaultProvider = "first"\nproviders = []', "providers"],
    ["duplicate ID", customConfig.replace('id = "second"', 'id = "first"'), "duplicate choice"],
    ["alias/ID collision", customConfig.replace('aliases = ["One"]', 'aliases = ["SECOND"]'), "duplicate choice"],
    ["alias collision", customConfig.replace('id = "second"', 'id = "second"\naliases = ["one"]'), "duplicate choice"],
    ["numeric alias", customConfig.replace('["One"]', '["1"]'), "textual aliases"],
    ["numeric ID", customConfig.replace('id = "first"', 'id = "1"'), ".id"],
    ["invalid enabled", customConfig.replaceAll("enabled = true", 'enabled = "true"'), ".enabled"],
    ["missing credential", customConfig.replace('credentialKey = "FIRST_TOKEN"', ""), ".credentialKey"],
    ["invalid credential key", customConfig.replace('"FIRST_TOKEN"', '"invalid key"'), ".credentialKey"],
    ["numeric environment", customConfig.replace('"983616"', "983616"), "string values"],
    ["invalid environment name", customConfig.replace("CUSTOM_PROVIDER_VALUE", '"INVALID=KEY"'), "environment names"],
    ["NUL environment", customConfig.replace('"first-only"', '"\\u0000"'), "invalid TOML"],
    ["inline credential", customConfig.replace("CUSTOM_PROVIDER_VALUE", "ANTHROPIC_AUTH_TOKEN"), "use credentialKey"],
    ["lowercase credential", customConfig.replace("CUSTOM_PROVIDER_VALUE", "anthropic_auth_token"), "use credentialKey"],
    ["unknown field", customConfig.replace('label = "First"', 'unknown = "First"'), "unknown field"],
    ["syntax", 'secret = "do-not-echo', "invalid TOML"],
  ])("rejects %s", (_name, text, error) => {
    expect(() => parseProviderConfig(text)).toThrow(error);
  });

  test("invalid configuration stops before provider selection or credentials", async () => {
    const calls: string[] = [];
    await expect(main([], {
      loadConfig: () => parseProviderConfig('secret = "do-not-echo'),
      selectProvider: async () => { calls.push("select"); return providerFromChoice("1", config); },
      resolveProviderCredential: async () => { calls.push("credential"); return "token"; },
      launchClaude: async () => { calls.push("launch"); return 0; },
    })).rejects.toThrow("cc.providers.toml: invalid TOML");
    expect(calls).toEqual([]);
  });

  test("reads the config beside the delivered script, independently of cwd", async () => {
    const dir = mkdtempSync(join(tmpdir(), "shine-cc-config-"));
    try {
      copyFileSync(join(import.meta.dir, "cc.ts"), join(dir, "cc.ts"));
      writeFileSync(join(dir, "cc.providers.toml"), customConfig);
      const probe = join(dir, "probe.ts");
      writeFileSync(probe, `import { loadProviderConfig } from "./cc.ts";
        console.log(JSON.stringify(loadProviderConfig()));`);
      const probeChild = Bun.spawn([process.execPath, "--no-install", probe], {
        cwd: tmpdir(), stdout: "pipe", stderr: "pipe",
      });
      const result = await new Response(probeChild.stdout).text();
      expect(await probeChild.exited).toBe(0);
      expect(await new Response(probeChild.stderr).text()).toBe("");
      expect(JSON.parse(result)).toEqual(parseProviderConfig(customConfig));
      const menu = Bun.spawn([process.execPath, "--no-install", join(dir, "cc.ts")], {
        cwd: tmpdir(), stdin: new TextEncoder().encode("invalid\n"), stdout: "pipe", stderr: "pipe",
      });
      const menuOutput = await new Response(menu.stdout).text();
      expect(await menu.exited).toBe(1);
      expect(menuOutput).toContain("1) First");
      expect(menuOutput).toContain("2) Second");
      expect(menuOutput).toContain("Provider [2]:");
      expect(await new Response(menu.stderr).text()).toContain("invalid provider");
      for (const content of [undefined, 'secret = "do-not-echo']) {
        if (content === undefined) rmSync(join(dir, "cc.providers.toml"));
        else writeFileSync(join(dir, "cc.providers.toml"), content);
        const child = Bun.spawn([process.execPath, "--no-install", join(dir, "cc.ts")], {
          cwd: tmpdir(), stdin: "ignore", stdout: "pipe", stderr: "pipe",
        });
        expect(await child.exited).toBe(1);
        expect(await new Response(child.stdout).text()).toBe("");
        const stderr = await new Response(child.stderr).text();
        expect(stderr).toContain("ccenv: cc.providers.toml:");
        expect(stderr).not.toContain("do-not-echo");
      }
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});

describe("provider selection", () => {
  const selections = [
    ["", "codex"],
    ["1", "codex"],
    ["Codex", "codex"],
    ["2", "deepseek"],
    ["DEEPSEEK", "deepseek"],
    ["3", "qwen"],
    ["Qwen", "qwen"],
  ] satisfies [string, string][];

  test.each(selections)("%s selects %s", (choice, expected) => {
    expect(providerFromChoice(choice, config).id).toBe(expected);
  });

  test("rejects unknown and disabled providers", () => {
    expect(() => providerFromChoice("other", config)).toThrow("invalid provider");
    expect(() => providerFromChoice("4", config)).toThrow("glm5 is not configured yet");
  });
});

describe("Claude arguments", () => {
  test("keeps ordinary arguments unchanged", () => {
    expect(claudeArgs(["--print", "hello world"])).toEqual([
      "--print",
      "hello world",
    ]);
  });

  test.each(["-r", "--run"])("%s remains a compatibility alias", (flag) => {
    expect(claudeArgs([flag])).toEqual([]);
  });

  test("double dash escapes a conflicting Claude argument", () => {
    expect(claudeArgs(["--", "--run"])).toEqual(["--run"]);
  });
});

function fakeRunner(
  values: Readonly<Record<string, string>>,
  decryptFailures: ReadonlySet<string> = new Set(),
): { runner: CommandRunner; calls: string[][] } {
  const calls: string[][] = [];
  return {
    calls,
    runner: async (args) => {
      calls.push([...args]);
      const getKey = args[2] ?? "";
      const decryptKey = args[3] ?? "";
      if (args[1] === "get" && getKey in values) {
        return { exitCode: 0, stdout: `${values[getKey]}\n` };
      }
      if (args[1] === "secret" && args[2] === "decrypt" && decryptKey in values) {
        return decryptFailures.has(decryptKey)
          ? { exitCode: 1, stdout: "" }
          : { exitCode: 0, stdout: `plain:${values[decryptKey]}` };
      }
      return { exitCode: 1, stdout: "" };
    },
  };
}

describe("credential resolution", () => {
  test("identifies the interactive secret decrypt command", () => {
    expect(isSecretDecryptCommand(["env", "secret", "decrypt", "TOKEN_SECRET"])).toBe(
      true,
    );
    expect(isSecretDecryptCommand(["env", "get", "TOKEN_SECRET"])).toBe(false);
  });

  test("prefers the generic tagged secret", async () => {
    const { runner, calls } = fakeRunner({
      TOKEN_SECRET: "age:ciphertext",
      TOKEN_GPG_SECRET: "legacy",
      TOKEN: "plaintext",
    });
    expect(await resolveCredential("TOKEN", runner)).toBe("plain:age:ciphertext");
    expect(calls).toEqual([
      ["env", "get", "TOKEN_SECRET"],
      ["env", "secret", "decrypt", "TOKEN_SECRET"],
    ]);
  });

  test("supports the legacy GPG secret before plaintext", async () => {
    const { runner } = fakeRunner({
      TOKEN_GPG_SECRET: "legacy",
      TOKEN: "plaintext",
    });
    expect(await resolveCredential("TOKEN", runner)).toBe("plain:legacy");
  });

  test("falls back to plaintext", async () => {
    const { runner } = fakeRunner({ TOKEN: "plaintext" });
    expect(await resolveCredential("TOKEN", runner)).toBe("plaintext");
  });

  test("does not fall back after decrypt failure", async () => {
    const { runner, calls } = fakeRunner(
      { TOKEN_SECRET: "broken", TOKEN: "plaintext" },
      new Set(["TOKEN_SECRET"]),
    );
    expect(resolveCredential("TOKEN", runner)).rejects.toThrow(
      "failed to decrypt TOKEN_SECRET",
    );
    expect(calls).not.toContainEqual(["env", "get", "TOKEN"]);
  });

  test("reports all accepted credential keys when missing", async () => {
    const { runner } = fakeRunner({});
    expect(resolveCredential("TOKEN", runner)).rejects.toThrow(
      "TOKEN_SECRET, TOKEN_GPG_SECRET, or TOKEN is not set",
    );
  });
});

describe("provider environments", () => {
  test("preserves configured environment names that match object properties", () => {
    const custom = parseProviderConfig(customConfig.replace(
      "CUSTOM_PROVIDER_VALUE", "__proto__",
    ));
    const environment = buildClaudeEnvironment(
      providerFromChoice("first", custom), "token", {}, custom.providers,
    );
    expect(Object.hasOwn(environment, "__proto__")).toBe(true);
    expect(environment["__proto__"]).toBe("first-only");
    expect(Object.getPrototypeOf(environment)).toBe(Object.prototype);
  });

  test("clears custom values owned by other providers without mutating the caller", () => {
    const custom = parseProviderConfig(customConfig);
    const base = { CUSTOM_PROVIDER_VALUE: "stale", PATH: "/bin", ANTHROPIC_API_KEY: "stale" };
    const environment = buildClaudeEnvironment(providerFromChoice("", custom), "token", base, custom.providers);
    expect(environment.CUSTOM_PROVIDER_VALUE).toBeUndefined();
    expect(environment.ANTHROPIC_API_KEY).toBeUndefined();
    expect(environment.CLAUDE_CODE_MAX_CONTEXT_TOKENS).toBe("983616");
    expect(environment.ANTHROPIC_AUTH_TOKEN).toBe("token");
    expect(base.CUSTOM_PROVIDER_VALUE).toBe("stale");
    expect(environment.PATH).toBe("/bin");
  });

  test.each(providers.filter((provider) => provider.enabled))(
    "$id clears stale provider values and applies its complete mapping",
    (provider) => {
      const environment = buildClaudeEnvironment(provider, "token", {
        PATH: "/bin",
        ANTHROPIC_API_KEY: "stale",
        ANTHROPIC_MODEL: "stale",
        CLAUDE_CODE_OAUTH_TOKEN: "stale",
        CLAUDE_CODE_EFFORT_LEVEL: "stale",
        CLAUDE_CODE_MAX_CONTEXT_TOKENS: "stale",
      });

      expect(environment.PATH).toBe("/bin");
      expect(environment.ANTHROPIC_AUTH_TOKEN).toBe("token");
      expect(environment.ANTHROPIC_API_KEY).toBeUndefined();
      expect(environment.CLAUDE_CODE_OAUTH_TOKEN).toBeUndefined();
      expect(environment).toMatchObject(provider.env);
      if (!("ANTHROPIC_MODEL" in provider.env)) {
        expect(environment.ANTHROPIC_MODEL).toBeUndefined();
      }
      if (!("CLAUDE_CODE_EFFORT_LEVEL" in provider.env)) {
        expect(environment.CLAUDE_CODE_EFFORT_LEVEL).toBeUndefined();
      }
      if (!("CLAUDE_CODE_MAX_CONTEXT_TOKENS" in provider.env)) {
        expect(environment.CLAUDE_CODE_MAX_CONTEXT_TOKENS).toBeUndefined();
      }
    },
  );
});

describe("launcher orchestration", () => {
  test("passes the provider environment, Claude arguments, and exit code", async () => {
    const provider = providerFromChoice("qwen", config);
    let launchedArgs: readonly string[] = [];
    let launchedEnvironment: Readonly<Record<string, string>> = {};
    const stdout = spyOn(console, "log").mockImplementation(() => {});
    const stderr = spyOn(console, "error").mockImplementation(() => {});

    try {
      const exitCode = await main(["--", "--run"], {
        loadConfig: () => config,
        selectProvider: async () => provider,
        resolveProviderCredential: async (key) => {
          expect(key).toBe("QWEN_API_KEY");
          return "test-token";
        },
        launchClaude: async (args, environment) => {
          launchedArgs = args;
          launchedEnvironment = environment;
          return 23;
        },
      });

      expect(exitCode).toBe(23);
      expect(launchedArgs).toEqual(["--run"]);
      expect(launchedEnvironment.ANTHROPIC_AUTH_TOKEN).toBe("test-token");
      expect(launchedEnvironment.ANTHROPIC_MODEL).toBe("qwen3.8-max");
      expect(stdout).toHaveBeenCalledWith(
        "ccenv: Claude Code environment configured for Qwen.",
      );
      expect(stderr).not.toHaveBeenCalled();
    } finally {
      stdout.mockRestore();
      stderr.mockRestore();
    }
  });
});
