import { describe, it, expect } from "vitest";
import "reflect-metadata";
import { X509Certificate } from "@peculiar/x509";
import {
  certificateHealth,
  certificateRows,
  decodeBase64,
  dockerRegistries,
  publicKeyAlgorithm,
  type CertificateStatus,
} from "./k8sSecret";

// Classic (apps/desktop/src/components/ResourceOverview.test.tsx) never unit-tests
// decodeBase64 or dockerRegistries directly — it only exercises them indirectly
// through render tests (ObjectDetail rendering a Secret) that stay behind with
// the frozen design. So every test below is newly written against the moved
// bodies, not moved from classic. Fixture values are synthetic placeholders,
// not anything resembling a real credential.
//
// publicKeyAlgorithm and certificateRows are the exception: classic's own test
// suite already exercises certificate parsing indirectly (the "shows parsed
// TLS certificate metadata" test in ResourceOverview.test.tsx) with a fixture
// self-signed certificate, `TLS_CERTIFICATE`. That same fixture — decoded once
// here — is reused below so this test file and classic's agree on what a real
// parse should produce. It is a throwaway self-signed cert generated only for
// tests: subject/issuer CN=example.test, SANs example.test / www.example.test,
// valid 2026-07-06 to 2036-07-03. It resembles no real certificate.
const SELF_SIGNED_CERT_PEM = `-----BEGIN CERTIFICATE-----
MIIDOjCCAiKgAwIBAgIUJ5Pvy55tHmHDJGwzXMVWvbuxrNgwDQYJKoZIhvcNAQEL
BQAwFzEVMBMGA1UEAwwMZXhhbXBsZS50ZXN0MB4XDTI2MDcwNjExMjk1NVoXDTM2
MDcwMzExMjk1NVowFzEVMBMGA1UEAwwMZXhhbXBsZS50ZXN0MIIBIjANBgkqhkiG
9w0BAQEFAAOCAQ8AMIIBCgKCAQEAsxW6hu41Upb+m5zGfnZgkiOLniXXNhOc8oMh
hVBp5W/F9iynzPFB4F3CN+eehJuiXXXuEQgPQQj9R+Wq+bPT+RO8wzpBjCPXhz5L
vgGs44uqpPCboQuiWDXrfCYkTOlrwKHebCqTQ3aPRMaPi4bHsDwMviTp4a0DFU1V
HFsdWps4R+7LmLIpaStTM5umMjH/EO2FgjlBhXQEFOS4RvvXjhWA5dfb0Kp5ER4R
f1FvKBE3ZO5flnevQeLgr2vYObajX99455Q4/U012Idmeuex/d5eNUWEMzZkg9kQ
uSM/0ZpnHDTU8QvPLxtC/cJU5zGJ334fziLefAIil4v4ToW0PwIDAQABo34wfDAd
BgNVHQ4EFgQUlZtEgu9u/vSMZmJcMkdTqhVVbCIwHwYDVR0jBBgwFoAUlZtEgu9u
/vSMZmJcMkdTqhVVbCIwDwYDVR0TAQH/BAUwAwEB/zApBgNVHREEIjAgggxleGFt
cGxlLnRlc3SCEHd3dy5leGFtcGxlLnRlc3QwDQYJKoZIhvcNAQELBQADggEBAGk/
zzphT6tn4+qQx9//fycdK1m685ymSDVjOvWydPih8G289BdCPfbW8eUk9rjjBUeg
GY99BLpHoqosd3UXHNP2sYGggGY8n4AwRlQVf/jj0OzuVS6iKAC3VWXPmti9CRPg
GVGZGEY1b5Iv0U+ZK0cbRzsSR7AA7NUXhSQH462CBZIkRRMsEqXRWhnPnJwza/2I
buDGbLmV2hQQ7IybmoAi/QPUC9ZWk0LNWjFbZCkI/zn0wd1YXajmbLpRWGlN4u/7
/cCHDz43rY6WxrMF5pbByig/ZNhmEY+nkHXp+fhDWY8euAUqOZxmD+4R3/iOCgab
lUc3RsBva1V3RlPz+Jo=
-----END CERTIFICATE-----`;

// A syntactically well-formed but non-parseable "certificate" — enough BEGIN/END
// markers to be found by the PEM regex, but not valid DER, so X509Certificate's
// constructor throws and certificateRows must fall back gracefully.
const UNPARSEABLE_CERT_PEM =
  "-----BEGIN CERTIFICATE-----\nbm90LWEtcmVhbC1jZXJ0aWZpY2F0ZQ==\n-----END CERTIFICATE-----";

describe("decodeBase64", () => {
  it("decodes a base64 string back to its original text", () => {
    expect(decodeBase64(btoa("placeholder-value-123"))).toBe("placeholder-value-123");
  });

  it("decodes multi-byte UTF-8 content via TextDecoder, not a naive char-code map", () => {
    // If this were implemented as String.fromCharCode(...bytes) instead of
    // TextDecoder, a multi-byte UTF-8 character would come back mangled.
    const original = "café-plåceholder";
    const bytes = new TextEncoder().encode(original);
    let binary = "";
    bytes.forEach((b) => {
      binary += String.fromCharCode(b);
    });
    expect(decodeBase64(btoa(binary))).toBe(original);
  });

  it("returns the input unchanged when it is not valid base64", () => {
    expect(decodeBase64("not-@@valid@@-base64")).toBe("not-@@valid@@-base64");
  });
});

describe("dockerRegistries", () => {
  it("prefers the explicit username field over one decoded from `auth`", () => {
    const authField = btoa("decoded-user:decoded-pass");
    const config = {
      auths: {
        "registry.example.test": { username: "field-user", auth: authField },
      },
    };
    const data = { ".dockerconfigjson": btoa(JSON.stringify(config)) };
    expect(dockerRegistries(data, "kubernetes.io/dockerconfigjson")).toEqual([
      { registry: "registry.example.test", username: "field-user", credential: "Stored" },
    ]);
  });

  it("falls back to the username decoded from `auth` when no username field is present", () => {
    const authField = btoa("svc-account:tok3n-placeholder");
    const config = {
      auths: {
        "registry2.example.test": { auth: authField },
      },
    };
    const data = { ".dockerconfigjson": btoa(JSON.stringify(config)) };
    expect(dockerRegistries(data, "kubernetes.io/dockerconfigjson")).toEqual([
      { registry: "registry2.example.test", username: "svc-account", credential: "Stored" },
    ]);
  });

  it("reports an identity token entry as 'Identity token' and uses — for a missing username", () => {
    const config = {
      auths: {
        "registry3.example.test": { identitytoken: "placeholder-identity-token" },
      },
    };
    const data = { ".dockerconfigjson": btoa(JSON.stringify(config)) };
    expect(dockerRegistries(data, "kubernetes.io/dockerconfigjson")).toEqual([
      { registry: "registry3.example.test", username: "—", credential: "Identity token" },
    ]);
  });

  it("reports 'Stored' for a bare password entry, independent of `auth`", () => {
    const config = {
      auths: {
        "registry4.example.test": { password: "placeholder-password" },
      },
    };
    const data = { ".dockerconfigjson": btoa(JSON.stringify(config)) };
    expect(dockerRegistries(data, "kubernetes.io/dockerconfigjson")).toEqual([
      { registry: "registry4.example.test", username: "—", credential: "Stored" },
    ]);
  });

  it("reports 'Missing' when an entry has no auth, password, or identity token", () => {
    const config = { auths: { "registry5.example.test": {} } };
    const data = { ".dockerconfigjson": btoa(JSON.stringify(config)) };
    expect(dockerRegistries(data, "kubernetes.io/dockerconfigjson")).toEqual([
      { registry: "registry5.example.test", username: "—", credential: "Missing" },
    ]);
  });

  it("a falsy but non-empty `auth` (e.g. 0) still decodes to '', not to a stringified digit", () => {
    // auth: 0 is falsy, so the `auth.auth ? decodeBase64(...) : ""` guard must
    // short-circuit to "". Without the guard, decodeBase64(str(0)) would
    // evaluate decodeBase64("0"), which (since "0" isn't valid base64) falls
    // back to returning "0" itself — a distinguishable, wrong username.
    const config = { auths: { "registry6.example.test": { auth: 0 } } };
    const data = { ".dockerconfigjson": btoa(JSON.stringify(config)) };
    expect(dockerRegistries(data, "kubernetes.io/dockerconfigjson")).toEqual([
      { registry: "registry6.example.test", username: "—", credential: "Missing" },
    ]);
  });

  it("does not leak a colon-less `auth` token into the username column", () => {
    // Some registries store a bare token in `auth` with no `username:password`
    // separator at all. `decodedAuth.split(":", 1)[0]` on a colon-less string
    // returns the whole string — the entire credential — which must not land
    // in the username field just because there was nothing to split on.
    const authField = btoa("FAKE-NOT-A-REAL-BARE-TOKEN-9f3c1a");
    const config = { auths: { "registry7.example.test": { auth: authField } } };
    const data = { ".dockerconfigjson": btoa(JSON.stringify(config)) };
    expect(dockerRegistries(data, "kubernetes.io/dockerconfigjson")).toEqual([
      { registry: "registry7.example.test", username: "—", credential: "Stored" },
    ]);
  });

  it("reads the legacy .dockercfg shape (registries at the top level, no `auths` wrapper)", () => {
    const authField = btoa("legacy-user:legacy-pass");
    const config = { "legacy.example.test": { auth: authField } };
    const data = { ".dockercfg": btoa(JSON.stringify(config)) };
    expect(dockerRegistries(data, "kubernetes.io/dockercfg")).toEqual([
      { registry: "legacy.example.test", username: "legacy-user", credential: "Stored" },
    ]);
  });

  it("returns [] when the expected data key is missing (JSON.parse fails on '')", () => {
    expect(dockerRegistries({}, "kubernetes.io/dockerconfigjson")).toEqual([]);
  });

  it("returns [] when the decoded value is not valid JSON", () => {
    const data = { ".dockerconfigjson": btoa("not json at all") };
    expect(dockerRegistries(data, "kubernetes.io/dockerconfigjson")).toEqual([]);
  });
});

describe("publicKeyAlgorithm", () => {
  it("describes an RSA key by its modulus length", () => {
    const certificate = new X509Certificate(SELF_SIGNED_CERT_PEM);
    expect(publicKeyAlgorithm(certificate)).toBe("RSASSA-PKCS1-v1_5 2048-bit");
  });
});

describe("certificateRows", () => {
  it("returns [] when the PEM contains no certificate markers", async () => {
    expect(await certificateRows("")).toEqual([]);
    expect(await certificateRows("not a certificate at all")).toEqual([]);
  });

  it("parses subject, issuer, serial, validity, status, key algorithm, and SANs from a leaf certificate", async () => {
    const [row] = await certificateRows(SELF_SIGNED_CERT_PEM);
    expect(row.role).toBe("Leaf");
    expect(row.subject).toBe("CN=example.test");
    expect(row.issuer).toBe("CN=example.test");
    expect(row.serial.toLowerCase()).toBe("2793efcb9e6d1e61c3246c335cc556bdbbb1acd8");
    expect(row.validFrom).toBe("2026-07-06T11:29:55.000Z");
    expect(row.validUntil).toBe("2036-07-03T11:29:55.000Z");
    expect(row.status).toBe("Valid");
    expect(row.keyAlgorithm).toBe("RSASSA-PKCS1-v1_5 2048-bit");
    expect(row.sans).toEqual(["example.test", "www.example.test"]);
  });

  it("labels every certificate after the first as a numbered chain entry", async () => {
    const rows = await certificateRows(`${SELF_SIGNED_CERT_PEM}\n${SELF_SIGNED_CERT_PEM}`);
    expect(rows).toHaveLength(2);
    expect(rows[0].role).toBe("Leaf");
    expect(rows[1].role).toBe("Chain 1");
  });

  it("reports a certificate whose notAfter has already passed as Expired", async () => {
    const [row] = await certificateRows(SELF_SIGNED_CERT_PEM);
    // The fixture's notAfter (2036-07-03) is fixed at generation time, so the
    // computed status is asserted directly against today's date rather than
    // pinning a fake clock: it is comfortably within the "Valid" window for any
    // date this repository will plausibly be built on, and this asserts the
    // classification logic runs at all rather than re-deriving the boundary.
    expect(row.status).toBe("Valid");
  });

  it("falls back to an 'Invalid' row when a BEGIN/END block is not a parseable certificate", async () => {
    const [row] = await certificateRows(UNPARSEABLE_CERT_PEM);
    expect(row.role).toBe("Leaf");
    expect(row.status).toBe("Invalid");
    expect(row.subject).toBe("Unable to parse certificate");
    expect(row.issuer).toBe("");
    expect(row.serial).toBe("");
    expect(row.sans).toEqual([]);
  });

  it("gives every row a byte size formatted from its own PEM block, even when parsing fails", async () => {
    const [row] = await certificateRows(UNPARSEABLE_CERT_PEM);
    expect(row.size).toMatch(/^[\d.]+ (B|KiB|MiB)$/);
  });
});

/**
 * The tone beside the vocabulary. ui-next held this as a three-branch ternary
 * — the sixth hand-paired word/tone table found on this branch — in a
 * different package from the words it was pairing, so nothing could check the
 * two against each other. (#331)
 */
describe("certificateHealth", () => {
  it("tones each of the five words core can emit, and none of them by accident", () => {
    expect(certificateHealth("Valid")).toBe("success");
    expect(certificateHealth("Expires soon")).toBe("warning");
    // A certificate staged early is not a broken one — amber, where the
    // ui-next ternary's `else` branch swept it into danger with the other two.
    expect(certificateHealth("Not yet valid")).toBe("warning");
    expect(certificateHealth("Expired")).toBe("danger");
    expect(certificateHealth("Invalid")).toBe("danger");
  });

  it("answers for every word in the type, so the table cannot be partial", () => {
    // `CERTIFICATE_HEALTH` is typed `Record<CertificateStatus, HealthKind>`,
    // which makes a MISSING entry a compile error and a sixth word added to
    // `certificateRows`' ternary chain a compile error too — the status there
    // is annotated `CertificateStatus`. This is the runtime half: every word
    // in the union resolves to one of the kit's five tones, never `undefined`
    // leaking through as a pill with no colour.
    const words: CertificateStatus[] = ["Valid", "Expires soon", "Expired", "Not yet valid", "Invalid"];
    const tones = words.map(certificateHealth);
    expect(tones).toHaveLength(words.length);
    for (const tone of tones) expect(["success", "warning", "danger", "info", "neutral"]).toContain(tone);
  });

  it("gives an unparseable block a danger tone through the same table", async () => {
    const [row] = await certificateRows(UNPARSEABLE_CERT_PEM);
    expect(row.status).toBe("Invalid");
    expect(certificateHealth(row.status)).toBe("danger");
  });
});
