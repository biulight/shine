import { describe, expect, test } from "bun:test";
import {
  SUBSCRIPTION_USER_AGENT,
  convertSubscription,
  isSurgeConfiguration,
} from "./generate-subscription";

function b64(value: string): string {
  return Buffer.from(value).toString("base64").replace(/=+$/, "");
}

function subscription(...uris: string[]): string {
  return b64(`${uris.join("\n")}\n`);
}

describe("Surge subscription conversion", () => {
  test("uses a neutral user agent so providers return URI subscriptions", () => {
    expect(SUBSCRIPTION_USER_AGENT).toBe("shine-subscription-generator/1");
    expect(SUBSCRIPTION_USER_AGENT.toLowerCase()).not.toContain("surge");
  });

  test("recognizes a complete Surge configuration before Base64 decoding", () => {
    expect(
      isSurgeConfiguration(
        "[General]\nloglevel = notify\n\n[Proxy]\nNode = direct\n",
      ),
    ).toBe(true);
    expect(isSurgeConfiguration(subscription("trojan://secret@example.com:443"))).toBe(
      false,
    );
  });

  test("reports a safe diagnostic for an invalid subscription URL", () => {
    const result = Bun.spawnSync({
      cmd: [process.execPath, "generate-subscription.ts"],
      cwd: import.meta.dir,
      env: {
        ...process.env,
        SURGE_SUBSCRIPTION_URL: "http://user:secret@example.invalid/subscription?token=secret",
      },
      stdout: "pipe",
      stderr: "pipe",
    });

    expect(result.exitCode).toBe(1);
    expect(result.stdout.toString()).toBe("");
    expect(result.stderr.toString()).toBe(
      "shine-generator-diagnostic-v1:https-required\n",
    );
    expect(result.stderr.toString()).not.toContain("secret");
  });

  test("converts SIP002 and legacy Shadowsocks URIs", () => {
    const sip = `ss://${b64("aes-128-gcm:secret")}@example.com:443#Tokyo`;
    const legacy = `ss://${b64("chacha20-ietf-poly1305:pwd@[2001:db8::1]:8443")}#IPv6`;
    const result = convertSubscription(subscription(sip, legacy));

    expect(result.output).toContain(
      "Tokyo = ss, example.com, 443, encrypt-method=aes-128-gcm, password=secret",
    );
    expect(result.output).toContain(
      "IPv6 = ss, 2001:db8::1, 8443, encrypt-method=chacha20-ietf-poly1305, password=pwd",
    );
    expect(result.stats.imported).toBe(2);
  });

  test("converts VMess TCP and WebSocket TLS fields", () => {
    const tcp = `vmess://${b64(JSON.stringify({
      ps: "TCP",
      add: "tcp.example",
      port: "80",
      id: "00000000-0000-0000-0000-000000000001",
      aid: "0",
      net: "tcp",
    }))}`;
    const ws = `vmess://${b64(JSON.stringify({
      ps: "WS",
      add: "ws.example",
      port: 443,
      id: "00000000-0000-0000-0000-000000000002",
      aid: 0,
      net: "ws",
      path: "/socket",
      host: "cdn.example",
      tls: "tls",
      sni: "origin.example",
    }))}`;
    const result = convertSubscription(subscription(tcp, ws));

    expect(result.output).toContain("TCP = vmess, tcp.example, 80");
    expect(result.output).toContain(
      "WS = vmess, ws.example, 443, username=00000000-0000-0000-0000-000000000002, vmess-aead=true, ws=true, ws-path=/socket, ws-headers=Host:cdn.example, tls=true, sni=origin.example",
    );
  });

  test("converts Trojan TCP and WebSocket URIs", () => {
    const tcp = "trojan://secret@example.com:443?security=tls&sni=origin.example#TCP";
    const ws = "trojan://p%40ss@[2001:db8::1]:8443?security=tls&type=ws&path=%2Fsocket&host=cdn.example&allowInsecure=1#WS";
    const result = convertSubscription(subscription(tcp, ws));

    expect(result.output).toContain(
      "TCP = trojan, example.com, 443, password=secret, sni=origin.example",
    );
    expect(result.output).toContain(
      "WS = trojan, 2001:db8::1, 8443, password=p@ss, ws=true, ws-path=/socket, ws-headers=Host:cdn.example, skip-cert-verify=true",
    );
    expect(result.stats.imported).toBe(2);
  });

  test("accepts an unencoded Trojan subscription", () => {
    const uri = "trojan://secret@example.com:443#Node";
    expect(convertSubscription(uri).output).toBe(
      "Node = trojan, example.com, 443, password=secret\n",
    );
  });

  test("skips VLESS, unsupported transports, invalid and duplicate nodes", () => {
    const ss = `ss://${b64("aes-128-gcm:secret")}@example.com:443#One`;
    const duplicate = `ss://${b64("aes-128-gcm:secret")}@example.com:443#Two`;
    const grpc = `vmess://${b64(JSON.stringify({
      ps: "gRPC",
      add: "grpc.example",
      port: 443,
      id: "00000000-0000-0000-0000-000000000003",
      net: "grpc",
    }))}`;
    const result = convertSubscription(
      subscription(ss, duplicate, "vless://example", grpc, "not-a-uri"),
    );

    expect(result.stats).toEqual({
      imported: 1,
      vless: 1,
      unsupported: 2,
      invalid: 0,
      duplicate: 1,
    });
    expect(result.output).not.toContain("vless");
  });

  test("uses stable suffixes for duplicate names", () => {
    const one = `ss://${b64("aes-128-gcm:one")}@one.example:443#Same`;
    const two = `ss://${b64("aes-128-gcm:two")}@two.example:443#Same`;
    const result = convertSubscription(subscription(one, two));

    expect(result.output).toContain("Same = ss");
    expect(result.output).toContain("Same (2) = ss");
  });

  test("accepts URL-safe outer base64 without padding", () => {
    const uri = `ss://${b64("aes-128-gcm:secret")}@example.com:443#Node`;
    const encoded = subscription(uri).replace(/\+/g, "-").replace(/\//g, "_");
    expect(convertSubscription(encoded).stats.imported).toBe(1);
  });

  test("rejects configuration delimiters and control characters in remote fields", () => {
    const good = `vmess://${b64(JSON.stringify({
      ps: "Good",
      add: "good.example",
      port: 443,
      id: "00000000-0000-0000-0000-000000000004",
    }))}`;
    const maliciousRecords = [
      { add: "bad.example\nInjected = direct" },
      { id: "uuid\r\nInjected = direct" },
      { net: "ws", path: "/socket\nInjected = direct" },
      { net: "ws", host: "cdn.example\u0000Injected" },
      { tls: "tls", sni: "origin.example\tInjected" },
      { add: "bad.example, direct" },
    ];
    const malicious = maliciousRecords.map((fields, index) =>
      `vmess://${b64(JSON.stringify({
        ps: `Bad ${index}`,
        add: "bad.example",
        port: 443,
        id: `00000000-0000-0000-0000-0000000001${index}`,
        ...fields,
      }))}`
    );

    const result = convertSubscription(subscription(good, ...malicious));

    expect(result.stats.imported).toBe(1);
    expect(result.stats.invalid).toBe(malicious.length);
    expect(result.output).toBe(
      "Good = vmess, good.example, 443, username=00000000-0000-0000-0000-000000000004, vmess-aead=true\n",
    );
    expect(result.output).not.toContain("Injected");
  });

  test("fails when no compatible nodes remain", () => {
    expect(() => convertSubscription(subscription("vless://example"))).toThrow(
      "no compatible proxy nodes",
    );
  });

  test("rejects unsupported or unsafe Trojan fields", () => {
    const good = "trojan://secret@good.example:443#Good";
    const bad = [
      "trojan://secret@bad.example:443?security=reality#Reality",
      "trojan://secret@bad.example:443?type=grpc#GRPC",
      "trojan://secret@bad.example:443?flow=xtls-rprx-vision#Flow",
      "trojan://secret@bad.example:443?type=ws&path=relative#Path",
      "trojan://bad%0AInjected%20%3D%20direct@bad.example:443#Password",
      "trojan://secret@bad.example:443?sni=bad.example%0AInjected%20%3D%20direct#SNI",
    ];
    const result = convertSubscription(subscription(good, ...bad));

    expect(result.stats.imported).toBe(1);
    expect(result.stats.unsupported).toBe(3);
    expect(result.stats.invalid).toBe(3);
    expect(result.output).toBe(
      "Good = trojan, good.example, 443, password=secret\n",
    );
    expect(result.output).not.toContain("Injected");
  });
});
