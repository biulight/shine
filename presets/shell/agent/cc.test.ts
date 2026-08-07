import { describe, expect, spyOn, test } from "bun:test";
import {
  buildClaudeEnvironment,
  claudeArgs,
  main,
  providerFromChoice,
  providers,
  isSecretDecryptCommand,
  resolveCredential,
  type CommandRunner,
  type ProviderId,
} from "./cc.ts";

describe("provider selection", () => {
  const selections = [
    ["", "codex"],
    ["1", "codex"],
    ["Codex", "codex"],
    ["2", "deepseek"],
    ["DEEPSEEK", "deepseek"],
    ["3", "qwen"],
    ["Qwen", "qwen"],
  ] satisfies [string, ProviderId][];

  test.each(selections)("%s selects %s", (choice, expected) => {
    expect(providerFromChoice(choice).id).toBe(expected);
  });

  test("rejects unknown and disabled providers", () => {
    expect(() => providerFromChoice("other")).toThrow("invalid provider");
    expect(() => providerFromChoice("4")).toThrow("glm5 is not configured yet");
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
    const provider = providerFromChoice("qwen");
    let launchedArgs: readonly string[] = [];
    let launchedEnvironment: Readonly<Record<string, string>> = {};
    const stdout = spyOn(console, "log").mockImplementation(() => {});
    const stderr = spyOn(console, "error").mockImplementation(() => {});

    try {
      const exitCode = await main(["--", "--run"], {
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
