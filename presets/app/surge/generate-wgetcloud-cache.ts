import { createHash } from "node:crypto";
import { stat } from "node:fs/promises";
import { homedir } from "node:os";
import { isAbsolute, join } from "node:path";

const MAX_CACHE_BYTES = 8 * 1024 * 1024;

function proxyList(profile: string): string | undefined {
  const proxies = new Map<string, string>();
  let section = "";

  for (const rawLine of profile.split(/\r\n|\n|\r/)) {
    const line = rawLine.trim();
    const header = /^(\[[^\]]+\])(?:[ \t]+(?:#|;|\/\/).*)?$/.exec(line)?.[1];
    if (header) {
      section = header;
      continue;
    }
    if (section !== "[Proxy]") continue;
    if (/^#!include\s/.test(line)) return undefined;
    if (line === "" || /^(#|;|\/\/)/.test(line)) continue;

    const separator = line.indexOf("=");
    if (separator < 0) return undefined;
    const name = line.slice(0, separator).trim();
    const definition = line.slice(separator + 1).trim();
    if (name === "" || definition === "" || proxies.has(name)) return undefined;
    proxies.set(name, rawLine);
  }

  if (proxies.size === 0) return undefined;
  return [...proxies.values()].join("\n") + "\n";
}

class CacheError extends Error {}

async function readConfig(path: string, readError: string, missingError = readError): Promise<Uint8Array> {
  let content: Uint8Array;
  try {
    const metadata = await stat(path);
    if (!metadata.isFile()) throw new CacheError(readError);
    if (metadata.size > MAX_CACHE_BYTES) throw new CacheError("cache-too-large");
    content = await Bun.file(path).slice(0, MAX_CACHE_BYTES + 1).bytes();
  } catch (error) {
    if (error instanceof CacheError) throw error;
    if ((error as NodeJS.ErrnoException | undefined)?.code === "ENOENT") {
      throw new CacheError(missingError);
    }
    throw new CacheError(readError);
  }
  if (content.byteLength > MAX_CACHE_BYTES) throw new CacheError("cache-too-large");
  return content;
}

function decodeConfig(content: Uint8Array, diagnostic: string): string {
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(content);
  } catch {
    throw new CacheError(diagnostic);
  }
}

async function profileCachePath(input: string, profilesDirectory: string): Promise<string> {
  if (!/^[^/\\\u0000-\u001f\u007f]+\.conf$/i.test(input)) {
    throw new CacheError("invalid-surge-cache-input");
  }
  const profile = await readConfig(join(profilesDirectory, input), "surge-profile-read-failed");
  const references = new Set<string>();
  let section = "";
  for (const rawLine of decodeConfig(profile, "surge-profile-read-failed").split(/\r\n|\n|\r/)) {
    const line = rawLine.trim().replace(/[ \t]+(?:#|;|\/\/).*$/, "").trimEnd();
    if (/^\[[^\]]+\]$/.test(line)) section = line;
    if (section !== "[Proxy]" && section !== "[Proxy Group]") continue;
    const reference = /^#!include[ \t]+(.+)$/.exec(line)?.[1];
    if (reference === undefined) continue;
    const quoted = /^"([^"]+)"$/.exec(reference)?.[1];
    if (quoted === undefined && reference.includes(",")) {
      throw new CacheError("surge-profile-reference-ambiguous");
    }
    references.add(quoted ?? reference);
  }
  if (references.size > 1) throw new CacheError("surge-profile-reference-ambiguous");
  const [reference] = references;
  if (!reference || !/^https?:\/\/[^\s"]+$/i.test(reference) || !URL.canParse(reference)) {
    throw new CacheError("surge-profile-reference-missing");
  }

  // Surge's managed-profile cache on macOS uses the original include URL as its key.
  const key = createHash("md5").update(reference, "utf8").digest("hex");
  return join(profilesDirectory, ".managed", `${key}.conf`);
}

export async function runCacheGenerator(input: string | undefined, profilesDirectory: string): Promise<void> {
  try {
    if (!input) throw new CacheError("missing-input");
    const direct = isAbsolute(input);
    const cachePath = direct ? input : await profileCachePath(input, profilesDirectory);
    const content = await readConfig(cachePath, "cache-read-failed", direct ? "cache-read-failed" : "surge-profile-cache-missing");
    const output = proxyList(decodeConfig(content, "invalid-surge-cache"));
    if (output === undefined) throw new CacheError("invalid-surge-cache");
    process.stdout.write(output);
  } catch (error) {
    const code = error instanceof CacheError ? error.message : "generation-failed";
    process.stderr.write(`shine-generator-diagnostic-v1:${code}\n`);
    process.exitCode = 1;
  }
}

if (import.meta.main) {
  await runCacheGenerator(
    process.env.W_GET_CLOUD_SURGE_CACHE_SOURCE,
    join(homedir(), "Library", "Application Support", "Surge", "Profiles"),
  );
}
