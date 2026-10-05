import { describe, it, expect } from "vitest";
import { ingressRuleAddress, ingressUsesRegexPaths } from "./ingressUrl";

const plain = { tlsHosts: [] as string[], regexPaths: false };

describe("ingressRuleAddress", () => {
  it("builds https for a host the Ingress terminates TLS for, and http otherwise", () => {
    expect(ingressRuleAddress("app.example.com", "/api", { ...plain, tlsHosts: ["app.example.com"] })).toEqual({
      kind: "url",
      url: "https://app.example.com/api",
    });
    expect(ingressRuleAddress("app.example.com", "/api", { ...plain, tlsHosts: ["www.example.com"] })).toEqual({
      kind: "url",
      url: "http://app.example.com/api",
    });
  });

  it("counts a wildcard TLS host as covering exactly one label", () => {
    const tls = { ...plain, tlsHosts: ["*.example.com"] };
    expect(ingressRuleAddress("api.example.com", "/", tls)).toEqual({ kind: "url", url: "https://api.example.com/" });
    expect(ingressRuleAddress("a.b.example.com", "/", tls)).toEqual({ kind: "url", url: "http://a.b.example.com/" });
    expect(ingressRuleAddress("example.com", "/", tls)).toEqual({ kind: "url", url: "http://example.com/" });
  });

  it("gives a wildcard host back as the host it is, not as an address", () => {
    expect(ingressRuleAddress("*.corp.dev", "/", plain)).toEqual({ kind: "host", host: "*.corp.dev" });
  });

  it("has nothing to offer for a rule with no host", () => {
    expect(ingressRuleAddress("", "/health", plain)).toBeNull();
  });

  it("links a regex path to the host's root", () => {
    expect(ingressRuleAddress("app.example.com", "/v(\\d+)/items", plain)).toEqual({
      kind: "url",
      url: "http://app.example.com/",
    });
    expect(ingressRuleAddress("app.example.com", "/static/*", plain)).toEqual({
      kind: "url",
      url: "http://app.example.com/",
    });
    // Under a regex-mode controller every path is a pattern, even one that
    // reads as a literal.
    expect(ingressRuleAddress("app.example.com", "/api", { ...plain, regexPaths: true })).toEqual({
      kind: "url",
      url: "http://app.example.com/",
    });
  });

  it("keeps a literal path's dots, and encodes what a URL path cannot carry", () => {
    expect(ingressRuleAddress("app.example.com", "/files/report.v2.txt", plain)).toEqual({
      kind: "url",
      url: "http://app.example.com/files/report.v2.txt",
    });
    expect(ingressRuleAddress("app.example.com", "/a b/ü", plain)).toEqual({
      kind: "url",
      url: "http://app.example.com/a%20b/%C3%BC",
    });
  });

  it("keeps a percent sign in a literal path as the character it is", () => {
    // A rule's path is matched against the decoded request path, so the
    // address must decode back to exactly the path as written. Set raw,
    // `%2e%2e` is read as `..` and the link went to `/admin`. (#797 review)
    expect(ingressRuleAddress("app.example.com", "/files/%2e%2e/admin", plain)).toEqual({
      kind: "url",
      url: "http://app.example.com/files/%252e%252e/admin",
    });
  });

  it("gives back as text a DNS name that a URL parser refuses or reads as an IP", () => {
    // Both pass DNS-1123 and the API server's host check, but a URL parser
    // reads a numeric last label as IPv4: `app.123` it refuses outright,
    // which threw out of the Rules section's render, and `123.123` it
    // re-reads as 123.0.0.123. (#797 review)
    expect(ingressRuleAddress("app.123", "/", plain)).toEqual({ kind: "host", host: "app.123" });
    expect(ingressRuleAddress("123.123", "/", plain)).toEqual({ kind: "host", host: "123.123" });
  });

  it("links a path that does not start with a slash to the host's root", () => {
    expect(ingressRuleAddress("app.example.com", "api", plain)).toEqual({ kind: "url", url: "http://app.example.com/" });
  });

  it("never turns a host that is not a DNS name into an address", () => {
    // `spec.rules[].host` is a DNS-1123 subdomain on any API server that
    // admitted the object; anything else came from somewhere it should not
    // have, and is shown, never opened.
    for (const host of [
      "evil.example@app.example.com",
      "app.example.com:8080",
      "app.example.com/admin",
      "App.Example.com",
      "app..example.com",
      "-app.example.com",
      "app.example.com.",
      "a b.example.com",
    ]) {
      expect(ingressRuleAddress(host, "/", plain), host).toEqual({ kind: "host", host });
    }
  });
});

describe("ingressUsesRegexPaths", () => {
  it("is on when ingress-nginx is told to treat paths as patterns", () => {
    expect(ingressUsesRegexPaths({ "nginx.ingress.kubernetes.io/use-regex": "true" })).toBe(true);
    // A rewrite target makes ingress-nginx match every path on the host as a
    // regex, whether or not use-regex says so.
    expect(ingressUsesRegexPaths({ "nginx.ingress.kubernetes.io/rewrite-target": "/$2" })).toBe(true);
  });

  it("is off otherwise", () => {
    expect(ingressUsesRegexPaths({})).toBe(false);
    expect(ingressUsesRegexPaths({ "nginx.ingress.kubernetes.io/use-regex": "false" })).toBe(false);
    expect(ingressUsesRegexPaths({ "kubernetes.io/ingress.class": "nginx" })).toBe(false);
  });
});
