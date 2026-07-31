// Launch Claude Code with a selected provider.
import { createInterface } from "node:readline/promises";

export type ProviderId = "codex" | "deepseek" | "qwen" | "glm5";

interface ProviderBase {
  id: ProviderId;
  label: string;
  aliases: readonly string[];
  env: Readonly<Record<string, string>>;
}

export interface EnabledProvider extends ProviderBase {
  enabled: true;
  credentialKey: string;
  configuredMessage: string;
}

export interface DisabledProvider extends ProviderBase {
  enabled: false;
}

export type Provider = EnabledProvider | DisabledProvider;

export const providers: readonly Provider[] = [
  {
    id: "codex",
    label: "codex",
    aliases: ["1", "codex"],
    credentialKey: "CLIPROXYAPI_AUTH_TOKEN",
    enabled: true,
    env: {
      ANTHROPIC_BASE_URL: "http://127.0.0.1:8317",
      ANTHROPIC_DEFAULT_OPUS_MODEL: "gpt-5.6-sol",
      ANTHROPIC_DEFAULT_SONNET_MODEL: "gpt-5.6-terra",
      ANTHROPIC_DEFAULT_HAIKU_MODEL: "gpt-5.6-luna",
      CLAUDE_CODE_SUBAGENT_MODEL: "gpt-5.6-luna",
      CLAUDE_CODE_EFFORT_LEVEL: "high",
    },
    configuredMessage:
      "Claude Code environment configured for Codex through CLIProxyAPI.",
  },
  {
    id: "deepseek",
    label: "deepseek",
    aliases: ["2", "deepseek"],
    credentialKey: "DEEPSEEK_API_KEY",
    enabled: true,
    env: {
      ANTHROPIC_BASE_URL: "https://api.deepseek.com/anthropic",
      ANTHROPIC_MODEL: "deepseek-v4-pro[1m]",
      ANTHROPIC_DEFAULT_OPUS_MODEL: "deepseek-v4-pro[1m]",
      ANTHROPIC_DEFAULT_SONNET_MODEL: "deepseek-v4-pro[1m]",
      ANTHROPIC_DEFAULT_HAIKU_MODEL: "deepseek-v4-flash",
      CLAUDE_CODE_SUBAGENT_MODEL: "deepseek-v4-flash",
      CLAUDE_CODE_EFFORT_LEVEL: "max",
    },
    configuredMessage: "Claude Code environment configured for DeepSeek.",
  },
  {
    id: "qwen",
    label: "qwen",
    aliases: ["3", "qwen"],
    credentialKey: "QWEN_API_KEY",
    enabled: true,
    env: {
      ANTHROPIC_BASE_URL:
        "https://token-plan.cn-beijing.maas.aliyuncs.com/apps/anthropic",
      ANTHROPIC_MODEL: "qwen3.8-max-preview",
      ANTHROPIC_DEFAULT_HAIKU_MODEL: "qwen3.6-flash",
      ANTHROPIC_DEFAULT_SONNET_MODEL: "qwen3.8-max-preview",
      ANTHROPIC_DEFAULT_OPUS_MODEL: "qwen3.8-max-preview",
      CLAUDE_CODE_SUBAGENT_MODEL: "qwen3.7-max",
      CLAUDE_CODE_MAX_CONTEXT_TOKENS: "983616",
    },
    configuredMessage: "Claude Code environment configured for Qwen.",
  },
  {
    id: "glm5",
    label: "glm5 (not configured yet)",
    aliases: ["4", "glm", "glm5"],
    enabled: false,
    env: {},
  },
];

const managedEnvironmentKeys = [
  "ANTHROPIC_API_KEY",
  "ANTHROPIC_AUTH_TOKEN",
  "ANTHROPIC_BASE_URL",
  "ANTHROPIC_MODEL",
  "ANTHROPIC_DEFAULT_OPUS_MODEL",
  "ANTHROPIC_DEFAULT_SONNET_MODEL",
  "ANTHROPIC_DEFAULT_HAIKU_MODEL",
  "CLAUDE_CODE_OAUTH_TOKEN",
  "CLAUDE_CODE_SUBAGENT_MODEL",
  "CLAUDE_CODE_EFFORT_LEVEL",
  "CLAUDE_CODE_MAX_CONTEXT_TOKENS",
] as const;

export interface CommandResult {
  exitCode: number;
  stdout: string;
}

export type CommandRunner = (args: readonly string[]) => Promise<CommandResult>;

async function runShine(args: readonly string[]): Promise<CommandResult> {
  try {
    const child = Bun.spawn(["shine", ...args], {
      stdout: "pipe",
      stderr: "ignore",
    });
    const [exitCode, stdout] = await Promise.all([
      child.exited,
      new Response(child.stdout).text(),
    ]);
    return { exitCode, stdout };
  } catch {
    throw new Error("the shine command was not found on PATH");
  }
}

async function readValue(
  key: string,
  runner: CommandRunner,
): Promise<string | undefined> {
  const result = await runner(["env", "get", key]);
  return result.exitCode === 0 ? result.stdout.replace(/\r?\n$/, "") : undefined;
}

async function decryptValue(
  key: string,
  runner: CommandRunner,
): Promise<string> {
  const result = await runner(["env", "decrypt", key]);
  if (result.exitCode !== 0) {
    throw new Error(`failed to decrypt ${key}`);
  }
  return result.stdout;
}

export async function resolveCredential(
  key: string,
  runner: CommandRunner = runShine,
): Promise<string> {
  for (const secretKey of [`${key}_SECRET`, `${key}_GPG_SECRET`]) {
    if ((await readValue(secretKey, runner)) !== undefined) {
      const value = await decryptValue(secretKey, runner);
      if (value.length === 0) {
        throw new Error(`${secretKey} decrypted to an empty value`);
      }
      return value;
    }
  }

  const plaintext = await readValue(key, runner);
  if (plaintext === undefined || plaintext.length === 0) {
    throw new Error(
      `${key}_SECRET, ${key}_GPG_SECRET, or ${key} is not set in the active shine env config`,
    );
  }
  return plaintext;
}

export function providerFromChoice(choice: string): EnabledProvider {
  const normalized = choice.trim().toLowerCase() || "1";
  const provider = providers.find((candidate) =>
    candidate.aliases.includes(normalized),
  );
  if (!provider) {
    throw new Error(`invalid provider: ${choice}`);
  }
  if (!provider.enabled) {
    throw new Error(`${provider.id} is not configured yet`);
  }
  return provider;
}

export function claudeArgs(args: readonly string[]): string[] {
  if (["-r", "--run", "--"].includes(args[0] ?? "")) {
    return args.slice(1);
  }
  return [...args];
}

export function buildClaudeEnvironment(
  provider: Provider,
  credential: string,
  baseEnvironment: Readonly<Record<string, string | undefined>> = Bun.env,
): Record<string, string> {
  const environment = Object.fromEntries(
    Object.entries(baseEnvironment).filter(
      (entry): entry is [string, string] => entry[1] !== undefined,
    ),
  );
  for (const key of managedEnvironmentKeys) {
    delete environment[key];
  }
  Object.assign(environment, provider.env, {
    ANTHROPIC_AUTH_TOKEN: credential,
  });
  return environment;
}

async function promptForProvider(): Promise<EnabledProvider> {
  console.log("Select Claude Code provider:");
  providers.forEach((provider, index) => {
    console.log(`  ${index + 1}) ${provider.label}`);
  });
  const readline = createInterface({
    input: process.stdin,
    output: process.stdout,
  });
  try {
    return providerFromChoice(await readline.question("Provider [1]: "));
  } finally {
    readline.close();
  }
}

export interface MainDependencies {
  selectProvider: () => Promise<EnabledProvider>;
  resolveProviderCredential: (key: string) => Promise<string>;
  launchClaude: (
    args: readonly string[],
    environment: Readonly<Record<string, string>>,
  ) => Promise<number>;
}

async function launchClaude(
  args: readonly string[],
  environment: Readonly<Record<string, string>>,
): Promise<number> {
  const child = Bun.spawn(["claude", ...args], {
    env: environment,
    stdin: "inherit",
    stdout: "inherit",
    stderr: "inherit",
  });
  return await child.exited;
}

const defaultDependencies: MainDependencies = {
  selectProvider: promptForProvider,
  resolveProviderCredential: resolveCredential,
  launchClaude,
};

export async function main(
  args: readonly string[] = Bun.argv.slice(2),
  dependencies: MainDependencies = defaultDependencies,
): Promise<number> {
  const provider = await dependencies.selectProvider();
  const credential = await dependencies.resolveProviderCredential(
    provider.credentialKey,
  );
  const environment = buildClaudeEnvironment(provider, credential);
  console.log(`ccenv: ${provider.configuredMessage}`);

  return await dependencies.launchClaude(claudeArgs(args), environment);
}

if (import.meta.main) {
  try {
    process.exit(await main());
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    console.error(`ccenv: ${message}`);
    process.exit(1);
  }
}
