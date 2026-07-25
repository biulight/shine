import { describe, expect, test } from "bun:test";
import { convertSubscription } from "./generate-subscription";

function b64(value: string): string {
  return Buffer.from(value).toString("base64").replace(/=+$/, "");
}

function subscription(...uris: string[]): string {
  return b64(`${uris.join("\n")}\n`);
}

describe("Surge subscription conversion", () => {
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
});
