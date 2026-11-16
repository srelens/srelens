import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import React from "react";

const { getManifestMock, applyManifestMock } = vi.hoisted(() => ({
  getManifestMock: vi.fn(),
  applyManifestMock: vi.fn(),
}));
vi.mock("@srelens/core/lib/manifest", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@srelens/core/lib/manifest")>();
  return { ...actual, getManifest: getManifestMock, applyManifest: applyManifestMock };
});
// CodeMirror needs real layout (unavailable in jsdom); stand in a controlled
// textarea that mirrors the editor's value/onChange/aria-label contract.
vi.mock("../ui/CodeEditor", () => ({
  // `copy` rides on a data attribute: the real control is the kit's, tested
  // there, and what matters at a CALL site is that the pane asked for one.
  CodeEditor: ({
    value,
    onChange,
    ariaLabel,
    copy,
    readOnly,
  }: {
    value: string;
    onChange?: (v: string) => void;
    ariaLabel?: string;
    copy?: boolean;
    readOnly?: boolean;
  }) => (
    <textarea
      aria-label={ariaLabel}
      value={value}
      readOnly={readOnly}
      onChange={(e) => onChange?.(e.target.value)}
      data-copy={String(!!copy)}
    />
  ),
}));
// ManifestEditor gates Apply (fail-closed) on a preflight access check in edit
// mode; stub the hook to "allowed" so these tests exercise the apply flow
// rather than the RBAC gate (covered in ManifestEditor.test.tsx).
vi.mock("@srelens/core/react", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@srelens/core/lib/access")>();
  return {
    ...actual,
    useAccess: () => ({ allowed: () => true, reason: () => "", known: () => true, loading: false }),
  };
});

import { YamlView } from "./YamlView";

/** A Secret's value, base64 as the API server returns it ("s3cret"). */
const SECRET_VALUE = "czNjcmV0";
/**
 * A Secret as `k8s.getManifest` returned one before the host redacted it, and
 * as it would again if that ever regressed: the value in `data`, and again in
 * the annotation `kubectl apply` writes.
 */
const SECRET_IN_THE_CLEAR = `apiVersion: v1
kind: Secret
metadata:
  name: api
  namespace: default
  annotations:
    kubectl.kubernetes.io/last-applied-configuration: '{"data":{"password":"${SECRET_VALUE}"}}'
type: Opaque
data:
  password: ${SECRET_VALUE}
`;

beforeEach(() => {
  getManifestMock.mockReset();
  applyManifestMock.mockReset();
});

describe("YamlView", () => {
  it("renders the fetched manifest in an editor", async () => {
    getManifestMock.mockResolvedValue({ yaml: "apiVersion: v1\nkind: Pod\n" });
    render(<YamlView context="kind-dev" kind="Pod" namespace="default" name="web-1" />);
    await waitFor(() =>
      expect((screen.getByLabelText("Manifest YAML") as HTMLTextAreaElement).value).toContain(
        "kind: Pod",
      ),
    );
    expect(getManifestMock).toHaveBeenCalledWith("kind-dev", "Pod", "default", "web-1", undefined, undefined);
  });

  it("offers to copy the manifest — a Secret's too, because what it copies is redacted", async () => {
    // #656 withheld Copy over a Secret here while this view showed one in the
    // clear (#659). The rule was never "no Copy near a Secret": it is "no Copy
    // over material the reader did not choose to see". Redacted on arrival,
    // the document carries none, so the Copy comes back — and the redaction is
    // asserted in the same breath, since a Copy over plaintext is the one
    // outcome this must never produce.
    getManifestMock.mockResolvedValue({ yaml: "kind: Pod" });
    const pod = render(<YamlView context="kind-dev" kind="Pod" namespace="default" name="web-1" />);
    expect((await pod.findByLabelText("Manifest YAML")).dataset.copy).toBe("true");
    pod.unmount();

    getManifestMock.mockResolvedValue({ yaml: SECRET_IN_THE_CLEAR });
    render(<YamlView context="kind-dev" kind="Secret" namespace="default" name="api" />);
    const secret = (await screen.findByLabelText("Manifest YAML")) as HTMLTextAreaElement;
    expect(secret.dataset.copy).toBe("true");
    expect(secret.value).not.toContain(SECRET_VALUE);
  });

  it("redacts a Secret's values on arrival, and says so", async () => {
    // `k8s.getManifest` is an ungated read. The host blanks a Secret's values
    // on it now (#661), but AGENTS.md asks the frontend to redact again on
    // arrival rather than trust that — this is the case where it has to.
    getManifestMock.mockResolvedValue({ yaml: SECRET_IN_THE_CLEAR });
    render(<YamlView context="kind-dev" kind="Secret" namespace="default" name="api" />);
    const editor = (await screen.findByLabelText("Manifest YAML")) as HTMLTextAreaElement;
    expect(editor.value).toContain("password: REDACTED");
    expect(editor.value).not.toContain(SECRET_VALUE);
    expect(document.body.textContent).not.toContain(SECRET_VALUE);
    // Told, not silently shown less: blank values read as an empty Secret.
    expect(screen.getByRole("status").textContent).toMatch(/Values redacted/);
  });

  it("keeps a Secret's manifest read-only, with no Apply to write the placeholders back", async () => {
    // What a current host sends: every value blanked (#661). From an editable
    // pane with a live Apply, one edited label later those blanks were written
    // over the Secret's real values and every annotation on it.
    getManifestMock.mockResolvedValue({
      yaml: 'apiVersion: v1\nkind: Secret\nmetadata:\n  name: api\n  namespace: default\ndata:\n  password: ""\n',
    });
    render(<YamlView context="kind-dev" kind="Secret" namespace="default" name="api" />);
    const editor = (await screen.findByLabelText("Manifest YAML")) as HTMLTextAreaElement;
    expect(editor.readOnly).toBe(true);
    expect(screen.queryByRole("button", { name: "Apply" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Reset" })).toBeNull();
  });

  it("leaves a custom kind that is merely NAMED Secret editable, and unredacted", async () => {
    // A CRD always has a group, so `crd` set means this is not the core
    // Secret — the host does not redact it either. Its drawer has no Overview
    // tab to send anyone to, and no reason to lose its Apply.
    getManifestMock.mockResolvedValue({
      yaml: `apiVersion: acme.io/v1\nkind: Secret\nmetadata:\n  name: api\n  namespace: default\ndata:\n  password: ${SECRET_VALUE}\n`,
    });
    render(
      <YamlView
        context="kind-dev"
        kind="Secret"
        namespace="default"
        name="api"
        crd={{ group: "acme.io", version: "v1", plural: "secrets" }}
      />,
    );
    const editor = (await screen.findByLabelText("Manifest YAML")) as HTMLTextAreaElement;
    expect(editor.value).toContain(`password: ${SECRET_VALUE}`);
    expect(editor.readOnly).toBe(false);
    expect(screen.getByRole("button", { name: "Apply" })).toBeDefined();
    expect(screen.queryByText(/Values redacted/)).toBeNull();
  });

  it("fails closed: a Secret manifest that cannot be redacted is not shown at all", async () => {
    // An alias can carry a redacted value somewhere the redactor did not
    // blank, so `redactSecretManifest` refuses the document outright.
    getManifestMock.mockResolvedValue({
      yaml: `apiVersion: v1\nkind: Secret\nmetadata:\n  name: api\ndata:\n  a: &v ${SECRET_VALUE}\n  b: *v\n`,
    });
    render(<YamlView context="kind-dev" kind="Secret" namespace="default" name="api" />);
    expect(await screen.findByText(/could not be redacted/)).toBeDefined();
    expect(screen.queryByLabelText("Manifest YAML")).toBeNull();
    expect(document.body.textContent).not.toContain(SECRET_VALUE);
  });

  it("shows a load error", async () => {
    getManifestMock.mockResolvedValue({ error: "not found" });
    render(<YamlView context="kind-dev" kind="Pod" namespace={null} name="x" />);
    await waitFor(() => expect(screen.getByText(/not found/)).toBeDefined());
  });

  it("edits and applies the manifest behind a confirm", async () => {
    getManifestMock.mockResolvedValue({ yaml: "kind: ConfigMap\n" });
    applyManifestMock.mockResolvedValue({
      applied: true,
      documents: [{ kind: "ConfigMap", name: "cm", applied: true }],
    });
    render(<YamlView context="kind-dev" kind="ConfigMap" namespace="default" name="cm" />);

    const textarea = await screen.findByLabelText("Manifest YAML");
    // Apply is disabled until edited.
    expect((screen.getByRole("button", { name: "Apply" }) as HTMLButtonElement).disabled).toBe(true);

    fireEvent.change(textarea, { target: { value: "kind: ConfigMap\ndata:\n  k: v\n" } });
    fireEvent.click(screen.getByRole("button", { name: "Apply" }));
    // confirm dialog — outside content is aria-hidden, so the dialog's Apply
    // is the only reachable one.
    expect(screen.getByRole("dialog")).toBeDefined();
    fireEvent.click(screen.getByRole("button", { name: "Apply" }));

    await waitFor(() =>
      expect(applyManifestMock).toHaveBeenCalledWith("kind-dev", "kind: ConfigMap\ndata:\n  k: v\n", false, "default"),
    );
  });
});
