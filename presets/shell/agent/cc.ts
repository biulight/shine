// Launch Claude Code with a selected provider.
import { readFileSync } from "node:fs";
import { createInterface } from "node:readline/promises";

interface ProviderBase {
  id: string;
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

export interface ProviderConfig {
  defaultProvider: string;
  providers: readonly Provider[];
}

const configFile = "cc.providers.toml";
const credentialEnvironmentKeys = [
  "ANTHROPIC_API_KEY",
  "ANTHROPIC_AUTH_TOKEN",
  "CLAUDE_CODE_OAUTH_TOKEN",
];

function configError(field: string): never {
  throw new Error(`${configFile}: invalid ${field}`);
}

function configTable(value: unknown, field: string): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    configError(field);
  }
  return value as Record<string, unknown>;
}

function configText(value: unknown, field: string): string {
  if (typeof value !== "string" || !value.trim() || /[\x00-\x1f\x7f]/.test(value)) {
    configError(field);
  }
  return value;
}

function checkFields(
  table: Record<string, unknown>,
  allowed: readonly string[],
  field: string,
): void {
  if (Object.keys(table).some((key) => !allowed.includes(key))) {
    configError(`${field} (unknown field)`);
  }
}

export function parseProviderConfig(text: string): ProviderConfig {
  let parsed: unknown;
  try {
    parsed = Bun.TOML.parse(text);
  } catch {
    // Parser diagnostics can include source values. Report only the config filename.
    throw new Error(`${configFile}: invalid TOML`);
  }
  const config = configTable(parsed, "configuration");
  checkFields(config, ["defaultProvider", "providers"], "configuration");
  const defaultProvider = configText(config.defaultProvider, "defaultProvider");
  if (!Array.isArray(config.providers) || config.providers.length === 0) {
    configError("providers (expected a non-empty array)");
  }
  const choices = new Set<string>();
  const providers = config.providers.map((value: unknown, index: number): Provider => {
    const field = `providers[${index + 1}]`;
    const entry = configTable(value, field);
    checkFields(entry, [
      "id", "label", "aliases", "enabled", "credentialKey", "configuredMessage", "env",
    ], field);
    const id = configText(entry.id, `${field}.id`);
    if (!/^[a-z][a-z0-9_-]*$/.test(id)) configError(`${field}.id`);
    const label = configText(entry.label, `${field}.label`);
    const aliases: unknown = entry.aliases ?? [];
    if (
      !Array.isArray(aliases) ||
      aliases.some((alias) =>
        typeof alias !== "string" || !/^[a-z][a-z0-9_-]*$/i.test(alias))
    ) {
      configError(`${field}.aliases (expected textual aliases)`);
    }
    for (const choice of [id, ...aliases]) {
      const normalized = choice.toLowerCase();
      if (choices.has(normalized)) {
        configError(`${field}.id/aliases (duplicate choice)`);
      }
      choices.add(normalized);
    }
    if (typeof entry.enabled !== "boolean") configError(`${field}.enabled`);
    const env = Object.fromEntries(
      Object.entries(configTable(entry.env ?? {}, `${field}.env`)).map(([key, value]) => {
        if (
          !/^[A-Za-z_][A-Za-z0-9_]*$/.test(key) ||
          typeof value !== "string" || value.includes("\0")
        ) {
          configError(`${field}.env (expected environment names and string values)`);
        }
        if (credentialEnvironmentKeys.includes(key.toUpperCase())) {
          configError(`${field}.env (use credentialKey for authentication)`);
        }
        return [key, value];
      }),
    );
    const base = { id, label, aliases: aliases as string[], env };
    const credentialKey = entry.credentialKey === undefined
      ? undefined
      : configText(entry.credentialKey, `${field}.credentialKey`);
    if (credentialKey !== undefined && !/^[A-Za-z_][A-Za-z0-9_]*$/.test(credentialKey)) {
      configError(`${field}.credentialKey`);
    }
    const configuredMessage = entry.configuredMessage === undefined
      ? `Claude Code environment configured for ${label}.`
      : configText(entry.configuredMessage, `${field}.configuredMessage`);
    if (!entry.enabled) return { ...base, enabled: false };
    if (credentialKey === undefined) configError(`${field}.credentialKey`);
    return { ...base, enabled: true, credentialKey, configuredMessage };
  });
  if (!providers.some((provider) => provider.id === defaultProvider && provider.enabled)) {
    configError("defaultProvider (expected an enabled provider ID)");
  }
  return { defaultProvider, providers };
}

export function loadProviderConfig(): ProviderConfig {
  let text: string;
  try {
    text = readFileSync(new URL(`./${configFile}`, import.meta.url), "utf8");
  } catch {
    throw new Error(`${configFile}: could not read configuration`);
  }
  return parseProviderConfig(text);
}

const managedEnvironmentKeys = [
  ...credentialEnvironmentKeys,
  "ANTHROPIC_BASE_URL",
  "ANTHROPIC_MODEL",
  "ANTHROPIC_DEFAULT_OPUS_MODEL",
  "ANTHROPIC_DEFAULT_SONNET_MODEL",
  "ANTHROPIC_DEFAULT_HAIKU_MODEL",
  "CLAUDE_CODE_SUBAGENT_MODEL",
  "CLAUDE_CODE_EFFORT_LEVEL",
  "CLAUDE_CODE_MAX_CONTEXT_TOKENS",
] as const;

export interface CommandResult {
  exitCode: number;
  stdout: string;
}

export type CommandRunner = (args: readonly string[]) => Promise<CommandResult>;

export function isSecretDecryptCommand(args: readonly string[]): boolean {
  return args[0] === "env" && args[1] === "secret" && args[2] === "decrypt";
}

async function runShine(args: readonly string[]): Promise<CommandResult> {
  try {
    const interactive = isSecretDecryptCommand(args);
    const child = Bun.spawn(["shine", ...args], {
      stdin: interactive ? "inherit" : "ignore",
      stdout: "pipe",
      // GPG/YubiKey may require a PIN and touch confirmation. Preserve the terminal
      // only for decrypt so credential probes stay quiet but authentication remains usable.
      stderr: interactive ? "inherit" : "ignore",
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
  const result = await runner(["env", "secret", "decrypt", key]);
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

export function providerFromChoice(
  choice: string,
  config: ProviderConfig,
): EnabledProvider {
  const normalized = choice.trim().toLowerCase();
  const provider = normalized === ""
    ? config.providers.find((candidate) => candidate.id === config.defaultProvider)
    : /^[1-9][0-9]*$/.test(normalized)
      ? config.providers[Number(normalized) - 1]
      : config.providers.find((candidate) =>
        candidate.id === normalized ||
        candidate.aliases.some((alias) => alias.toLowerCase() === normalized));
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
  providers: readonly Provider[] = [provider],
): Record<string, string> {
  const environment = Object.fromEntries(
    Object.entries(baseEnvironment).filter(
      (entry): entry is [string, string] => entry[1] !== undefined,
    ),
  );
  const keys = [
    ...managedEnvironmentKeys,
    ...providers.flatMap((item) => Object.keys(item.env)),
  ];
  for (const key of keys) {
    delete environment[key];
  }
  return {
    ...environment,
    ...provider.env,
    ANTHROPIC_AUTH_TOKEN: credential,
  };
}

async function promptForProvider(config: ProviderConfig): Promise<EnabledProvider> {
  console.log("Select Claude Code provider:");
  config.providers.forEach((provider, index) => {
    console.log(`  ${index + 1}) ${provider.label}`);
  });
  const readline = createInterface({
    input: process.stdin,
    output: process.stdout,
  });
  try {
    const defaultNumber = config.providers.findIndex(
      (provider) => provider.id === config.defaultProvider,
    ) + 1;
    return providerFromChoice(
      await readline.question(`Provider [${defaultNumber}]: `), config,
    );
  } finally {
    readline.close();
  }
}

export interface MainDependencies {
  loadConfig: () => ProviderConfig;
  selectProvider: (config: ProviderConfig) => Promise<EnabledProvider>;
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
  loadConfig: loadProviderConfig,
  selectProvider: promptForProvider,
  resolveProviderCredential: resolveCredential,
  launchClaude,
};

export async function main(
  args: readonly string[] = Bun.argv.slice(2),
  dependencies: MainDependencies = defaultDependencies,
): Promise<number> {
  const config = dependencies.loadConfig();
  const provider = await dependencies.selectProvider(config);
  const credential = await dependencies.resolveProviderCredential(
    provider.credentialKey,
  );
  const environment = buildClaudeEnvironment(
    provider, credential, Bun.env, config.providers,
  );
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
