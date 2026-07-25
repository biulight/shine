const MAX_RESPONSE_BYTES = 8 * 1024 * 1024;
const REQUEST_TIMEOUT_MS = 20_000;

export interface ConversionStats {
  imported: number;
  vless: number;
  unsupported: number;
  invalid: number;
  duplicate: number;
}

export interface ConversionResult {
  output: string;
  stats: ConversionStats;
}

type ParsedProxy = {
  name: string;
  signature: string;
  definition: string;
};

function decodeBase64(input: string): string {
  const normalized = input
    .replace(/\s+/g, "")
    .replace(/-/g, "+")
    .replace(/_/g, "/");
  const padded = normalized.padEnd(
    normalized.length + ((4 - (normalized.length % 4)) % 4),
    "=",
  );
  if (!/^[A-Za-z0-9+/]*={0,2}$/.test(padded)) {
    throw new Error("invalid base64");
  }
  return Buffer.from(padded, "base64").toString("utf8");
}

function decodeOuterSubscription(input: string): string {
  const trimmed = input.trim();
  if (/^(ss|vmess|vless):\/\//m.test(trimmed)) {
    return trimmed;
  }
  return decodeBase64(trimmed);
}

function decodeName(value: string | undefined, fallback: string): string {
  if (!value) return fallback;
  try {
    return decodeURIComponent(value);
  } catch {
    return fallback;
  }
}

function sanitizeName(value: string): string {
  const cleaned = value
    .replace(/[\r\n=,]/g, " ")
    .replace(/\s+/g, " ")
    .trim();
  return (cleaned || "Subscription Node").slice(0, 96);
}

function value(value: string): string {
  if (/^[^,\r\n"]+$/.test(value)) return value;
  return `"${value.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`;
}

function parsePort(value: string | number): number {
  const port = Number(value);
  if (!Number.isInteger(port) || port < 1 || port > 65_535) {
    throw new Error("invalid port");
  }
  return port;
}

function parseHostPort(input: string): { host: string; port: number } {
  if (input.startsWith("[")) {
    const end = input.indexOf("]");
    if (end < 0 || input[end + 1] !== ":") throw new Error("invalid IPv6");
    return {
      host: input.slice(1, end),
      port: parsePort(input.slice(end + 2)),
    };
  }
  const separator = input.lastIndexOf(":");
  if (separator <= 0) throw new Error("missing port");
  return {
    host: input.slice(0, separator),
    port: parsePort(input.slice(separator + 1)),
  };
}

function parseCredentials(input: string): { method: string; password: string } {
  const separator = input.indexOf(":");
  if (separator <= 0) throw new Error("invalid credentials");
  const method = input.slice(0, separator);
  const password = input.slice(separator + 1);
  if (!password) throw new Error("empty password");
  return { method, password };
}

function parseShadowsocks(uri: string): ParsedProxy {
  const body = uri.slice("ss://".length);
  const hashAt = body.indexOf("#");
  const withoutHash = hashAt >= 0 ? body.slice(0, hashAt) : body;
  const fragment = hashAt >= 0 ? body.slice(hashAt + 1) : undefined;
  const queryAt = withoutHash.indexOf("?");
  const authority = queryAt >= 0 ? withoutHash.slice(0, queryAt) : withoutHash;
  const query = queryAt >= 0 ? new URLSearchParams(withoutHash.slice(queryAt + 1)) : null;
  if (query?.has("plugin")) throw new Error("unsupported plugin");

  let credentials: { method: string; password: string };
  let endpoint: { host: string; port: number };
  const at = authority.lastIndexOf("@");
  if (at >= 0) {
    const encodedCredentials = authority.slice(0, at);
    const decodedCredentials = encodedCredentials.includes(":")
      ? decodeURIComponent(encodedCredentials)
      : decodeBase64(encodedCredentials);
    credentials = parseCredentials(decodedCredentials);
    endpoint = parseHostPort(authority.slice(at + 1));
  } else {
    const legacy = decodeBase64(authority);
    const legacyAt = legacy.lastIndexOf("@");
    if (legacyAt <= 0) throw new Error("invalid legacy URI");
    credentials = parseCredentials(legacy.slice(0, legacyAt));
    endpoint = parseHostPort(legacy.slice(legacyAt + 1));
  }

  const fallback = `SS ${endpoint.host}:${endpoint.port}`;
  const name = sanitizeName(decodeName(fragment, fallback));
  const params = [
    "ss",
    endpoint.host,
    String(endpoint.port),
    `encrypt-method=${value(credentials.method)}`,
    `password=${value(credentials.password)}`,
  ];
  return {
    name,
    signature: params.join(", "),
    definition: params.join(", "),
  };
}

function asString(record: Record<string, unknown>, key: string): string {
  const raw = record[key];
  return raw === undefined || raw === null ? "" : String(raw);
}

function parseVmess(uri: string): ParsedProxy {
  const record = JSON.parse(
    decodeBase64(uri.slice("vmess://".length)),
  ) as Record<string, unknown>;
  const host = asString(record, "add").trim();
  const port = parsePort(asString(record, "port"));
  const id = asString(record, "id").trim();
  if (!host || !id) throw new Error("missing VMess endpoint");

  const network = asString(record, "net").trim().toLowerCase() || "tcp";
  if (network !== "tcp" && network !== "ws") {
    throw new Error("unsupported VMess transport");
  }

  const params = ["vmess", host, String(port), `username=${value(id)}`];
  const alterId = asString(record, "aid").trim();
  params.push(`vmess-aead=${alterId === "" || alterId === "0" ? "true" : "false"}`);

  const cipher = asString(record, "scy").trim().toLowerCase();
  if (
    cipher === "aes-128-gcm" ||
    cipher === "chacha20-ietf-poly1305"
  ) {
    params.push(`encrypt-method=${cipher}`);
  }

  if (network === "ws") {
    params.push("ws=true");
    const path = asString(record, "path");
    if (path) params.push(`ws-path=${value(path)}`);
    const wsHost = asString(record, "host");
    if (wsHost) params.push(`ws-headers=${value(`Host:${wsHost}`)}`);
  }

  const tls = asString(record, "tls").trim().toLowerCase();
  if (tls === "tls") {
    params.push("tls=true");
    const sni = asString(record, "sni").trim() || asString(record, "host").trim();
    if (sni) params.push(`sni=${value(sni)}`);
    if (/^(1|true)$/i.test(asString(record, "allowInsecure"))) {
      params.push("skip-cert-verify=true");
    }
  } else if (tls && tls !== "none") {
    throw new Error("unsupported VMess security");
  }

  const name = sanitizeName(asString(record, "ps") || `VMess ${host}:${port}`);
  return {
    name,
    signature: params.join(", "),
    definition: params.join(", "),
  };
}

export function convertSubscription(input: string): ConversionResult {
  const stats: ConversionStats = {
    imported: 0,
    vless: 0,
    unsupported: 0,
    invalid: 0,
    duplicate: 0,
  };
  const decoded = decodeOuterSubscription(input);
  const seen = new Set<string>();
  const names = new Map<string, number>();
  const definitions: string[] = [];

  for (const rawLine of decoded.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (!line) continue;
    if (line.startsWith("vless://")) {
      stats.vless += 1;
      continue;
    }

    let parsed: ParsedProxy;
    try {
      if (line.startsWith("ss://")) {
        parsed = parseShadowsocks(line);
      } else if (line.startsWith("vmess://")) {
        parsed = parseVmess(line);
      } else {
        stats.unsupported += 1;
        continue;
      }
    } catch (error) {
      const message = error instanceof Error ? error.message : "";
      if (message.startsWith("unsupported")) stats.unsupported += 1;
      else stats.invalid += 1;
      continue;
    }

    if (seen.has(parsed.signature)) {
      stats.duplicate += 1;
      continue;
    }
    seen.add(parsed.signature);
    const occurrence = (names.get(parsed.name) ?? 0) + 1;
    names.set(parsed.name, occurrence);
    const uniqueName = occurrence === 1 ? parsed.name : `${parsed.name} (${occurrence})`;
    definitions.push(`${uniqueName} = ${parsed.definition}`);
  }

  stats.imported = definitions.length;
  if (stats.imported === 0) throw new Error("no compatible proxy nodes");
  return {
    output: `${definitions.join("\n")}\n`,
    stats,
  };
}

function summary(stats: ConversionStats): string {
  const skipped = [
    `vless=${stats.vless}`,
    `unsupported=${stats.unsupported}`,
    `invalid=${stats.invalid}`,
    `duplicate=${stats.duplicate}`,
  ].join(", ");
  return `surge subscription: imported ${stats.imported}; skipped ${skipped}`;
}

async function readLimitedResponse(response: Response): Promise<Uint8Array> {
  if (!response.body) return new Uint8Array();
  const reader = response.body.getReader();
  const chunks: Uint8Array[] = [];
  let total = 0;
  for (;;) {
    const { done, value: chunk } = await reader.read();
    if (done) break;
    total += chunk.byteLength;
    if (total > MAX_RESPONSE_BYTES) {
      await reader.cancel();
      throw new Error("response too large");
    }
    chunks.push(chunk);
  }
  const bytes = new Uint8Array(total);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return bytes;
}

async function main(): Promise<void> {
  try {
    const configured = process.env.SURGE_SUBSCRIPTION_URL;
    if (!configured) throw new Error("missing URL");
    const url = new URL(configured);
    if (url.protocol !== "https:") throw new Error("HTTPS required");

    const response = await fetch(url, {
      redirect: "follow",
      signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS),
      headers: { "user-agent": "shine-surge-generator/1" },
    });
    if (!response.ok) throw new Error("request failed");
    if (new URL(response.url).protocol !== "https:") {
      throw new Error("HTTPS redirect required");
    }
    const declaredLength = Number(response.headers.get("content-length") ?? "0");
    if (declaredLength > MAX_RESPONSE_BYTES) throw new Error("response too large");
    const bytes = await readLimitedResponse(response);

    const result = convertSubscription(new TextDecoder().decode(bytes));
    process.stdout.write(result.output);
    const skipped =
      result.stats.vless +
      result.stats.unsupported +
      result.stats.invalid +
      result.stats.duplicate;
    if (skipped > 0) process.stderr.write(`${summary(result.stats)}\n`);
  } catch {
    process.stderr.write("surge subscription: generation failed (details redacted)\n");
    process.exitCode = 1;
  }
}

if (import.meta.main) {
  await main();
}
