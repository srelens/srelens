import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { K8sObject, PodMetric, PodSummary } from "@srelens/core";

// `SecretDetailsBody` reads the real (base64) values via core's gated
// `getSecret` (`getObject` redacts Secret data) — mocked here so a test
// controls what "the cluster said" without one. A Secret has no
// `relatedPodSelector` match, so `podsForSelector`/`podMetrics` are mocked
// only so the composition test can render `GenericBody` without a live
// cluster call escaping the mock boundary. `importOriginal` keeps every
// formatter (`decodeBase64`, `dockerRegistries`, `formatBytes`, ...) intact.
const { getSecret, podsForSelector, podMetrics } = vi.hoisted(() => ({
  getSecret: vi.fn(async (): Promise<{ data?: Record<string, string>; error?: string }> => ({ data: undefined })),
  podsForSelector: vi.fn(async (): Promise<{ pods?: PodSummary[]; error?: string }> => ({ pods: [] })),
  podMetrics: vi.fn(async (): Promise<{ metrics?: PodMetric[]; error?: string }> => ({ metrics: [] })),
}));

vi.mock("@srelens/core", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@srelens/core")>()),
  getSecret,
  podsForSelector,
  podMetrics,
}));

import { GenericBody } from "./GenericBody";
import { detailFacts } from "./detailData";
import { SecretDetailsBody } from "./SecretBody";

// Obviously-fake fixture text — never anything that reads as a real
// credential, per the task's secrecy ruling.
const FIXTURE_VALUE = "fixture-only-not-a-real-secret";
const FIXTURE_B64 = btoa(FIXTURE_VALUE);

function secret(
  data: Record<string, string> = {},
  overrides: Partial<K8sObject> = {},
  metadata: NonNullable<K8sObject["metadata"]> = { name: "s-1", namespace: "default" },
): K8sObject {
  return { kind: "Secret", apiVersion: "v1", metadata, data, ...overrides } as K8sObject;
}

/** Scans the whole rendered document for a substring — text content, `title`,
 *  `aria-label`, `data-*`, everything, including markup a screen reader or a
 *  DOM inspector would see even while visually hidden. A boolean assertion
 *  rather than an element query, so a failure here never prints the secret
 *  text into the test output. */
function documentContains(value: string): boolean {
  return document.body.innerHTML.includes(value);
}

describe("SecretDetailsBody", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    getSecret.mockResolvedValue({ data: undefined });
    podsForSelector.mockResolvedValue({ pods: [] });
    podMetrics.mockResolvedValue({ metrics: [] });
  });

  describe("the secrecy ruling", () => {
    it("keeps a fetched value out of the document until it is revealed, then shows it, then hides it again", async () => {
      getSecret.mockResolvedValue({ data: { token: FIXTURE_B64 } });
      render(<SecretDetailsBody object={secret({ token: "" })} context="ctx" />);

      await waitFor(() => expect(getSecret).toHaveBeenCalledWith("ctx", "default", "s-1"));
      // The fetched value is in memory now, but must not be in the document —
      // not as text, not as a title/aria-label/data-* attribute, and not in
      // the accessibility tree behind a visually-hidden style.
      expect(documentContains(FIXTURE_VALUE)).toBe(false);
      expect(documentContains(FIXTURE_B64)).toBe(false);
      expect(screen.queryByText(FIXTURE_VALUE)).toBeNull();

      await userEvent.click(screen.getByRole("button", { name: "Reveal" }));
      await waitFor(() => expect(documentContains(FIXTURE_VALUE)).toBe(true));

      await userEvent.click(screen.getByRole("button", { name: "Hide" }));
      expect(documentContains(FIXTURE_VALUE)).toBe(false);
    });

    it("never exposes the value through the toggle button's own accessible name", async () => {
      getSecret.mockResolvedValue({ data: { token: FIXTURE_B64 } });
      render(<SecretDetailsBody object={secret({ token: "" })} context="ctx" />);
      await waitFor(() => expect(getSecret).toHaveBeenCalled());
      const toggle = screen.getByRole("button", { name: "Reveal" });
      expect(toggle.getAttribute("title")).toBeNull();
      expect(toggle.getAttribute("aria-label")).toBeNull();
    });

    it("shows the masked placeholder, not an empty value, before reveal", async () => {
      getSecret.mockResolvedValue({ data: { token: FIXTURE_B64 } });
      render(<SecretDetailsBody object={secret({ token: "" })} context="ctx" />);
      await waitFor(() => expect(getSecret).toHaveBeenCalled());
      expect(screen.getByText("••••••••")).toBeDefined();
    });

    it("falls back to the redacted (blank) keys while the fetch is still in flight", () => {
      getSecret.mockImplementation(() => new Promise(() => {}));
      render(<SecretDetailsBody object={secret({ token: "" })} context="ctx" />);
      expect(screen.getByText("token")).toBeDefined();
      expect(screen.getByText("••••••••")).toBeDefined();
    });
  });

  describe("a general (Opaque) secret", () => {
    it("shows the Secret summary: type, key count, and immutable", async () => {
      render(<SecretDetailsBody object={secret({ token: "" }, { type: "Opaque", immutable: true })} context="ctx" />);
      await waitFor(() => expect(getSecret).toHaveBeenCalled());
      expect(screen.getByText("Secret summary")).toBeDefined();
      expect(screen.getByText("Opaque")).toBeDefined();
      expect(screen.getByText("1 key")).toBeDefined();
      expect(screen.getByText("Yes")).toBeDefined();
    });

    it("defaults type to Opaque and immutable to No when absent", async () => {
      render(<SecretDetailsBody object={secret({})} context="ctx" />);
      await waitFor(() => expect(getSecret).toHaveBeenCalled());
      expect(screen.getByText("Opaque")).toBeDefined();
      expect(screen.getByText("No")).toBeDefined();
    });

    it("shows an empty state for a secret with no data", async () => {
      render(<SecretDetailsBody object={secret({})} context="ctx" />);
      await waitFor(() => expect(getSecret).toHaveBeenCalled());
      expect(screen.getByText("Data (0 keys)")).toBeDefined();
      expect(screen.getByText("No data")).toBeDefined();
    });

    it("does not render TLS material or Docker registries sections", async () => {
      render(<SecretDetailsBody object={secret({ token: "" })} context="ctx" />);
      await waitFor(() => expect(getSecret).toHaveBeenCalled());
      expect(screen.queryByText("TLS material")).toBeNull();
      expect(screen.queryByText("Docker registries")).toBeNull();
    });
  });

  describe("a kubernetes.io/tls secret", () => {
    const CERT =
      "-----BEGIN CERTIFICATE-----\nFAKE-NOT-A-REAL-CERTIFICATE\n-----END CERTIFICATE-----\n";
    const KEY = "-----BEGIN PRIVATE KEY-----\nFAKE-NOT-A-REAL-KEY\n-----END PRIVATE KEY-----\n";

    // A real, parseable self-signed certificate generated only for tests — the
    // same fixture core's k8sSecret.test.ts and classic's
    // ResourceOverview.test.tsx use, so every test suite that parses a
    // certificate agrees on what a real parse should produce. It resembles no
    // real certificate: CN=example.test, SANs example.test / www.example.test,
    // valid 2026-07-06 to 2036-07-03.
    const PARSEABLE_CERT = `-----BEGIN CERTIFICATE-----
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

    it("shows the certificate count, private key format, and encoded sizes", async () => {
      render(
        <SecretDetailsBody
          object={secret({ "tls.crt": btoa(CERT), "tls.key": btoa(KEY) }, { type: "kubernetes.io/tls" })}
          context="ctx"
        />,
      );
      await waitFor(() => expect(getSecret).toHaveBeenCalled());
      expect(screen.getByText("TLS material")).toBeDefined();
      expect(screen.getByText("kubernetes.io/tls")).toBeDefined();
      expect(screen.getByText("1 certificate")).toBeDefined();
      expect(screen.getByText("PKCS#8")).toBeDefined();
    });

    it("reports a missing certificate rather than a count of zero", async () => {
      render(
        <SecretDetailsBody object={secret({ "tls.key": btoa(KEY) }, { type: "kubernetes.io/tls" })} context="ctx" />,
      );
      await waitFor(() => expect(getSecret).toHaveBeenCalled());
      expect(screen.getByText("Missing tls.crt")).toBeDefined();
    });

    it("reports a missing private key", async () => {
      render(
        <SecretDetailsBody object={secret({ "tls.crt": btoa(CERT) }, { type: "kubernetes.io/tls" })} context="ctx" />,
      );
      await waitFor(() => expect(getSecret).toHaveBeenCalled());
      expect(screen.getByText("Missing")).toBeDefined();
    });

    it("still shows the Data section with tls.crt and tls.key masked", async () => {
      render(
        <SecretDetailsBody
          object={secret({ "tls.crt": btoa(CERT), "tls.key": btoa(KEY) }, { type: "kubernetes.io/tls" })}
          context="ctx"
        />,
      );
      await waitFor(() => expect(getSecret).toHaveBeenCalled());
      expect(screen.getByText("tls.crt")).toBeDefined();
      expect(screen.getByText("tls.key")).toBeDefined();
      expect(screen.getAllByText("••••••••")).toHaveLength(2);
      expect(documentContains(CERT)).toBe(false);
      expect(documentContains(KEY)).toBe(false);
    });

    describe("certificate facts (parsed via core's certificateRows)", () => {
      it("shows status, subject, issuer, serial, public key algorithm, validity, and SANs from the leaf certificate", async () => {
        render(
          <SecretDetailsBody
            object={secret(
              { "tls.crt": btoa(PARSEABLE_CERT), "tls.key": btoa(KEY) },
              { type: "kubernetes.io/tls" },
            )}
            context="ctx"
          />,
        );
        await waitFor(() => expect(getSecret).toHaveBeenCalled());
        // Subject, Issuer (equal, self-signed) and the per-certificate
        // table's Subject column all show "CN=example.test".
        await waitFor(() => expect(screen.getAllByText("CN=example.test")).toHaveLength(3));
        expect(screen.getByText("2793efcb9e6d1e61c3246c335cc556bdbbb1acd8")).toBeDefined();
        expect(screen.getByText("RSASSA-PKCS1-v1_5 2048-bit")).toBeDefined();
        expect(screen.getByText("example.test")).toBeDefined();
        expect(screen.getByText("www.example.test")).toBeDefined();
        // Both a "Certificate status" KV row and the per-certificate table's
        // own Status column show "Valid" for this fixture's validity window.
        expect(screen.getAllByText("Valid").length).toBeGreaterThanOrEqual(2);
      });

      it("renders the per-certificate table with role, subject, status, and size", async () => {
        render(
          <SecretDetailsBody
            object={secret(
              { "tls.crt": btoa(PARSEABLE_CERT), "tls.key": btoa(KEY) },
              { type: "kubernetes.io/tls" },
            )}
            context="ctx"
          />,
        );
        await waitFor(() => expect(getSecret).toHaveBeenCalled());
        await waitFor(() => expect(screen.getByText("Leaf")).toBeDefined());
        const table = screen.getByRole("table");
        expect(table).toBeDefined();
      });

      it("falls back to an Invalid status and no serial/issuer/SANs when the certificate fails to parse, without exposing the private key", async () => {
        render(
          <SecretDetailsBody
            object={secret({ "tls.crt": btoa(CERT), "tls.key": btoa(KEY) }, { type: "kubernetes.io/tls" })}
            context="ctx"
          />,
        );
        await waitFor(() => expect(getSecret).toHaveBeenCalled());
        // The "Certificate status" KV row and the per-certificate table's
        // Status column both show "Invalid".
        await waitFor(() => expect(screen.getAllByText("Invalid")).toHaveLength(2));
        // The "Subject" KV row and the table's Subject column both show it.
        expect(screen.getAllByText("Unable to parse certificate")).toHaveLength(2);
        expect(documentContains(KEY)).toBe(false);
        expect(documentContains("FAKE-NOT-A-REAL-KEY")).toBe(false);
      });
    });
  });

  describe("a kubernetes.io/dockerconfigjson secret", () => {
    const DOCKER_CONFIG = JSON.stringify({
      auths: { "registry.example.test": { username: "robot", password: "fixture-only-not-real" } },
    });

    it("summarises registries without exposing the password", async () => {
      render(
        <SecretDetailsBody
          object={secret(
            { ".dockerconfigjson": btoa(DOCKER_CONFIG) },
            { type: "kubernetes.io/dockerconfigjson" },
          )}
          context="ctx"
        />,
      );
      await waitFor(() => expect(getSecret).toHaveBeenCalled());
      expect(screen.getByText("Docker registries")).toBeDefined();
      expect(screen.getByText("registry.example.test")).toBeDefined();
      expect(screen.getByText("robot")).toBeDefined();
      expect(screen.getByText("Stored")).toBeDefined();
      expect(documentContains("fixture-only-not-real")).toBe(false);
    });

    it("shows an empty state when no valid registry credentials are found", async () => {
      render(
        <SecretDetailsBody
          object={secret({ ".dockerconfigjson": btoa("not json") }, { type: "kubernetes.io/dockerconfigjson" })}
          context="ctx"
        />,
      );
      await waitFor(() => expect(getSecret).toHaveBeenCalled());
      expect(screen.getByText("No valid registry credentials found")).toBeDefined();
    });
  });

  it("is a run of flat blocks, not a stack of cards", async () => {
    const { container } = render(<SecretDetailsBody object={secret({ token: "" })} context="ctx" />);
    await waitFor(() => expect(getSecret).toHaveBeenCalled());
    const blocks = [...container.children];
    expect(blocks).toHaveLength(2);
    for (const block of blocks) expect(block.matches("section.section")).toBe(true);
    expect(container.querySelector(".card")).toBeNull();
  });

  describe("composition with GenericBody", () => {
    it("states the Secret's namespace once and renders no Pods section", async () => {
      const s = secret({ token: "" });
      render(
        <GenericBody kind="Secret" object={s} context="ctx">
          <SecretDetailsBody object={s} context="ctx" />
        </GenericBody>,
      );
      await waitFor(() => expect(getSecret).toHaveBeenCalled());
      // The namespace is a fact the SCREEN draws, off the one derivation both
      // screens read; neither wrapper nor body states it again.
      expect(detailFacts({ kind: "Secret", object: s }).map((f) => f.label)).toContain("Namespace");
      expect(screen.queryByText("Namespace")).toBeNull();
      expect(screen.queryAllByRole("heading", { name: "Pods" })).toHaveLength(0);
      expect(podsForSelector).not.toHaveBeenCalled();
    });
  });
});

// `k8s.getSecret` is its own SENSITIVE_READ capability with its own timeout, so
// a consent refusal, a 403 scoped to `secrets`, and a timeout are all ordinary
// paths. The fixture below is the only one that can express what happens on
// them: `getObject` SUCCEEDED — so `object.data` carries the real KEY NAMES
// with every value blanked to `""` by `redact_secret_data` — while `getSecret`
// FAILED. Put real values in `object.data` (as the suites above do, for
// brevity) and the defect is unreachable, because the fallback then happens to
// hold the truth.
describe("when the values could not be read (getSecret refused, getObject's blanks are all there is)", () => {
  const REFUSED = "consent denied for k8s.getSecret";

  beforeEach(() => {
    vi.clearAllMocks();
    getSecret.mockResolvedValue({ error: REFUSED });
    podsForSelector.mockResolvedValue({ pods: [] });
    podMetrics.mockResolvedValue({ metrics: [] });
  });

  /** A Secret as `getObject` returns one: keys intact, every value blanked. */
  const blanked = (keys: string[], overrides: Partial<K8sObject> = {}) =>
    secret(Object.fromEntries(keys.map((k) => [k, ""])), overrides);

  it("says the values could not be read rather than leaving the reader to read the blanks as facts", async () => {
    render(<SecretDetailsBody object={blanked(["token"])} context="ctx" />);
    await waitFor(() => expect(screen.getByText("This Secret's values could not be read")).toBeDefined());
  });

  it("offers the refusal the cluster actually gave, folded away", async () => {
    render(<SecretDetailsBody object={blanked(["token"])} context="ctx" />);
    await waitFor(() => expect(screen.getByText("This Secret's values could not be read")).toBeDefined());
    expect(document.body.textContent).toContain(REFUSED);
  });

  it("withholds Reveal rather than offering a toggle onto an empty pre", async () => {
    render(<SecretDetailsBody object={blanked(["token"])} context="ctx" />);
    await waitFor(() => expect(screen.getByText("This Secret's values could not be read")).toBeDefined());
    const toggle = screen.getByRole("button", { name: "Reveal" });
    // Enabled, this button decodes `""` and shows an empty `<pre>`: a key the
    // reader may not read looks identical to a key whose value is empty.
    expect((toggle as HTMLButtonElement).disabled).toBe(true);
    await userEvent.click(toggle);
    expect(screen.getByText("••••••••")).toBeDefined();
  });

  it("still names every key it does know — the key names survived the redaction", async () => {
    render(<SecretDetailsBody object={blanked(["token", "ca.crt"])} context="ctx" />);
    await waitFor(() => expect(screen.getByText("This Secret's values could not be read")).toBeDefined());
    expect(screen.getByText("Data (2 keys)")).toBeDefined();
    expect(screen.getByText("token")).toBeDefined();
    expect(screen.getByText("ca.crt")).toBeDefined();
  });

  it("does not state a TLS Secret that has tls.crt and tls.key as Missing", async () => {
    render(
      <SecretDetailsBody
        object={blanked(["tls.crt", "tls.key"], { type: "kubernetes.io/tls" })}
        context="ctx"
      />,
    );
    await waitFor(() => expect(screen.getByText("This Secret's values could not be read")).toBeDefined());
    expect(screen.getByText("TLS material")).toBeDefined();
    // The Secret HAS both. Neither row may claim otherwise; the whole point is
    // that the pane does not know, and rows it cannot answer are omitted.
    expect(screen.queryByText("Missing tls.crt")).toBeNull();
    expect(screen.queryByText("Missing")).toBeNull();
    expect(screen.queryByText("Certificates")).toBeNull();
    expect(screen.queryByText("Private key")).toBeNull();
  });

  it("does not state a Docker Secret as 0 registries with no valid credentials", async () => {
    render(
      <SecretDetailsBody
        object={blanked([".dockerconfigjson"], { type: "kubernetes.io/dockerconfigjson" })}
        context="ctx"
      />,
    );
    await waitFor(() => expect(screen.getByText("This Secret's values could not be read")).toBeDefined());
    expect(screen.getByText("Docker registries")).toBeDefined();
    expect(screen.queryByText("0 registries")).toBeNull();
    expect(screen.queryByText("No valid registry credentials found")).toBeNull();
    expect(screen.queryByText("Registries")).toBeNull();
  });

  // The peek renders this body while the fetch is still in flight, so the gate
  // has to cover `loading` by exactly the same argument: the pane does not
  // know yet either.
  it("withholds the same assertions while the fetch is still in flight", () => {
    getSecret.mockImplementation(() => new Promise(() => {}));
    render(
      <SecretDetailsBody
        object={blanked(["tls.crt", "tls.key"], { type: "kubernetes.io/tls" })}
        context="ctx"
      />,
    );
    expect(screen.queryByText("Missing tls.crt")).toBeNull();
    expect(screen.queryByText("Missing")).toBeNull();
    // Both keys — tls.crt and tls.key.
    const toggles = screen.getAllByRole("button", { name: "Reveal" }) as HTMLButtonElement[];
    expect(toggles).toHaveLength(2);
    for (const t of toggles) expect(t.disabled).toBe(true);
    // ...and does not say the read FAILED, because it has not.
    expect(screen.queryByText("This Secret's values could not be read")).toBeNull();
  });

  it("also reads a rejected promise as a failure, not as an empty Secret", async () => {
    getSecret.mockRejectedValue(new Error(REFUSED));
    render(<SecretDetailsBody object={blanked(["token"])} context="ctx" />);
    await waitFor(() => expect(screen.getByText("This Secret's values could not be read")).toBeDefined());
  });
});
