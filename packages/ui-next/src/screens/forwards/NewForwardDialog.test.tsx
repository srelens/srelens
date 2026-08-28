import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

/**
 * **`forwardAddress`, `toKubectl` and `describeError` stay REAL.** They are the
 * three rules this dialog is forbidden from restating — where a forward is
 * reachable from, what the equivalent command reads, and how a failure is
 * worded — and a test that stubbed them would assert the dialog calls a stub.
 * The platform is flipped one layer lower instead, on the same module
 * `forward.ts` reads `isTauri` from, so `forwardAddress` computes for real on
 * both platforms. Same arrangement as `Forwards.test.tsx`.
 */
const platform = vi.hoisted(() => ({ isTauri: vi.fn(() => true) }));
vi.mock("@srelens/core/platform", async (orig) => ({
  ...(await orig<typeof import("@srelens/core/platform")>()),
  isTauri: platform.isTauri,
}));

/** The forwards store is core's, and it is what the clash check reads. */
const store = vi.hoisted(() => ({
  list: [] as unknown[],
  listeners: new Set<() => void>(),
}));
const core = vi.hoisted(() => ({
  startPortForward: vi.fn(),
  openExternal: vi.fn(),
  listNamespaces: vi.fn(),
  listPods: vi.fn(),
  listServices: vi.fn(),
  notify: { success: vi.fn(), error: vi.fn(), info: vi.fn() },
}));
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  getForwards: () => store.list,
  subscribeForwards: (l: () => void) => {
    store.listeners.add(l);
    return () => store.listeners.delete(l);
  },
  ...core,
}));

import { type ActiveForward, toKubectl } from "@srelens/core";
import { NewForwardDialog, OFFER_HIGH, OFFER_LOW, offerLocalPort } from "./NewForwardDialog";

const CONTEXT = "prod-eu";

/** §A.4's own namespaces. */
const NAMESPACES = ["checkout", "payments", "observability", "identity"];

/** §A.4's own targets, split across the two kinds that can back a forward. */
const SERVICES = [
  { name: "checkout-api", namespace: "checkout" },
  { name: "checkout-web", namespace: "checkout" },
];
const PODS = [{ name: "checkout-api-5c8b7f2d9-mk3wl", namespace: "checkout" }];

/** A live forward holding port 8080, so a clash has something to clash WITH. */
function holding(localPort: number): ActiveForward {
  return {
    id: 1,
    context: "prod-eu",
    namespace: "checkout",
    kind: "Service",
    name: "checkout-web",
    localPort,
    remotePort: 80,
    status: "active",
    bytesMoved: 0,
    startedAt: Date.now(),
  };
}

/**
 * A forward that GAVE UP: the row core keeps on screen after a tunnel
 * exhausted its retries. `failed` is the one status `isForwardEnded` calls
 * ended, and the backend's task — which owned the `TcpListener` — is gone with
 * it, so the local port is free even though the row is still listed.
 */
function dead(localPort: number): ActiveForward {
  return { ...holding(localPort), id: 2, status: "failed", error: "connection refused" };
}

/**
 * A forward that is between attempts. Still alive, still holding its listener:
 * `AttemptCtx` (and the socket in it) lives across the reconnect loop's
 * attempts, so this port is NOT free. The half that keeps `dead` above honest —
 * without it, "exclude what is not active" would pass the same assertion.
 */
function flapping(localPort: number): ActiveForward {
  return { ...holding(localPort), id: 3, status: "reconnecting", error: "connection reset" };
}

let opened: ReturnType<typeof vi.fn>;

beforeEach(() => {
  vi.clearAllMocks();
  platform.isTauri.mockReturnValue(true);
  core.listNamespaces.mockResolvedValue({ namespaces: NAMESPACES });
  core.listServices.mockResolvedValue({ services: SERVICES });
  core.listPods.mockResolvedValue({ pods: PODS });
  core.startPortForward.mockImplementation(async (req: { localPort?: number }) => ({
    id: 7,
    localPort: req.localPort ?? 0,
  }));
  // A forwards store with something in it by default: a clash test whose
  // fixture has no live forwards cannot fail, and neither can the "no clash"
  // half of it.
  store.list = [holding(8080)];
  store.listeners.clear();
  core.openExternal.mockResolvedValue(undefined);
  // Not the mechanism any more — the spy is here to prove it is NOT reached.
  // `window.open` is a silent no-op inside a Tauri WebView (#348), so a switch
  // that called it would look wired up and open nothing on the desktop.
  opened = vi.fn();
  Object.defineProperty(window, "open", { value: opened, configurable: true, writable: true });
});

function open(onClose = vi.fn()) {
  render(<NewForwardDialog context={CONTEXT} onClose={onClose} />);
  return onClose;
}

const field = (name: string) => screen.getByLabelText(name) as HTMLElement;
const startButton = () => screen.getByRole("button", { name: "Start forward" }) as HTMLButtonElement;

/** Wait for the namespace list to land, then fill §A.4's four fields. */
async function fillIn({ local = "9090", remote = "8080", target = "svc/checkout-api" } = {}) {
  await waitFor(() => expect(core.listNamespaces).toHaveBeenCalled());
  await userEvent.selectOptions(field("Namespace"), "checkout");
  await waitFor(() => expect(core.listServices).toHaveBeenCalled());
  await waitFor(() =>
    expect(within(field("Target")).queryByRole("option", { name: target })).toBeTruthy(),
  );
  await userEvent.selectOptions(field("Target"), target);
  await userEvent.clear(field("Local port"));
  if (local) await userEvent.type(field("Local port"), local);
  await userEvent.clear(field("Remote port"));
  if (remote) await userEvent.type(field("Remote port"), remote);
}

describe("NewForwardDialog — §A.4's frame", () => {
  it("is the design's 480px dialog with §A.4's four fields and its two controls", async () => {
    open();
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("New port forward")).toBeTruthy();
    expect((dialog as HTMLElement).style.maxWidth).toBe("480px");
    for (const label of ["Target", "Namespace", "Local port", "Remote port"]) {
      expect(field(label)).toBeTruthy();
    }
    expect(screen.getByRole("switch", { name: "Open in browser when it comes up" })).toBeTruthy();
    expect(screen.getByText("Equivalent command")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeTruthy();
    expect(startButton()).toBeTruthy();
  });

  it("closes on Cancel without starting anything", async () => {
    const onClose = open();
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(core.startPortForward).not.toHaveBeenCalled();
  });
});

describe("NewForwardDialog — what it offers to forward", () => {
  it("lists the cluster's namespaces, from core", async () => {
    open();
    await waitFor(() => expect(core.listNamespaces).toHaveBeenCalledWith(CONTEXT));
    await waitFor(() =>
      expect(within(field("Namespace")).getByRole("option", { name: "observability" })).toBeTruthy(),
    );
    // All four, not just the one the assertion above happened to name.
    for (const ns of NAMESPACES) {
      expect(within(field("Namespace")).getByRole("option", { name: ns })).toBeTruthy();
    }
  });

  it("names the chosen namespace's services and pods the way kubectl does", async () => {
    open();
    await waitFor(() => expect(core.listNamespaces).toHaveBeenCalled());
    await userEvent.selectOptions(field("Namespace"), "checkout");
    await waitFor(() => expect(core.listServices).toHaveBeenCalledWith(CONTEXT, "checkout"));
    expect(core.listPods).toHaveBeenCalledWith(CONTEXT, "checkout");
    const target = field("Target");
    await waitFor(() =>
      expect(within(target).getByRole("option", { name: "svc/checkout-api" })).toBeTruthy(),
    );
    // The prefix is read off the kind: a Pod is `pod/`, not `svc/`.
    expect(within(target).getByRole("option", { name: "svc/checkout-web" })).toBeTruthy();
    expect(
      within(target).getByRole("option", { name: "pod/checkout-api-5c8b7f2d9-mk3wl" }),
    ).toBeTruthy();
    expect(within(target).queryByRole("option", { name: "svc/checkout-api-5c8b7f2d9-mk3wl" })).toBeNull();
  });

  it("asks for a cluster instead of listing an empty context's namespaces", async () => {
    render(<NewForwardDialog context="" onClose={vi.fn()} />);
    await screen.findByRole("dialog");
    expect(
      within(field("Namespace")).getByRole("option", { name: "Nothing to list without a cluster" }),
    ).toBeTruthy();
    // The point: nothing goes to the backend to be told there is no cluster.
    expect(core.listNamespaces).not.toHaveBeenCalled();
    expect(startButton().disabled).toBe(true);
  });

  it("says there is no cluster up front, and does not send the reader to the rail for one", async () => {
    // The dialog is pinned to the cluster it was OPENED against, so a reader
    // who follows "pick a cluster in the rail first" changes nothing about this
    // dialog — it stays pinned to no cluster and can never start. It used to
    // arm the divergence banner instead, which then named neither side. Said
    // here, once, with the way out being to open it again.
    render(<NewForwardDialog context="" onClose={vi.fn()} />);
    await screen.findByRole("dialog");

    const alert = screen.getByText("No cluster in focus").closest("[data-tone]");
    expect(alert).toBeTruthy();
    expect(alert?.textContent).toMatch(/open New forward again/i);
    // No instruction anywhere that the reader could follow and be worse off.
    const dialog = screen.getByRole("dialog");
    expect(dialog.textContent).not.toMatch(/in the rail first/i);
  });

  it("says nothing about a missing cluster when it has one", async () => {
    open();
    await screen.findByRole("dialog");
    await waitFor(() => expect(core.listNamespaces).toHaveBeenCalledWith(CONTEXT));
    expect(screen.queryByText("No cluster in focus")).toBeNull();
  });

  it("says so when the namespaces cannot be listed, in words rather than in Rust", async () => {
    // `listNamespaces` REPORTS by returning an error field; it does not throw,
    // and a try/catch around it would catch nothing.
    core.listNamespaces.mockResolvedValue({
      error: "ApiError: Unauthorized (Status { metadata: Some(ListMeta { .. }) })",
    });
    open();
    expect(await screen.findByText(/rejected your credentials/i)).toBeTruthy();
    // The struct appears ONLY inside the disclosure, never as the sentence.
    const alert = screen.getByText(/rejected your credentials/i).closest("[data-tone]");
    const raw = alert?.querySelector('[data-slot="raw"]');
    expect(raw?.textContent).toContain("ApiError");
    expect(screen.getByText("Could not list this cluster's namespaces")).toBeTruthy();
  });
});

describe("NewForwardDialog — the port clash", () => {
  it("refuses a local port another forward already holds, in §A.4's words", async () => {
    open();
    await fillIn({ local: "8080" });
    expect(screen.getByText("Port 8080 is already forwarded.")).toBeTruthy();
    expect(startButton().disabled).toBe(true);
    // And it is decided HERE: no request goes out to be refused.
    expect(core.startPortForward).not.toHaveBeenCalled();
  });

  it("lets a free port through — the same form, one digit apart", async () => {
    // The half that makes the assertion above mean something: a Start button
    // that were disabled always would pass the clash test on its own.
    open();
    await fillIn({ local: "8081" });
    expect(screen.queryByText(/already forwarded/)).toBeNull();
    expect(startButton().disabled).toBe(false);
  });

  it("frees the port again when the forward holding it goes", async () => {
    open();
    await fillIn({ local: "8080" });
    expect(startButton().disabled).toBe(true);
    // The store is pushed to, and the clash is read off it live.
    store.list = [];
    for (const l of store.listeners) l();
    await waitFor(() => expect(startButton().disabled).toBe(false));
    expect(screen.queryByText(/already forwarded/)).toBeNull();
  });

  it("will not start without both ports", async () => {
    open();
    await fillIn({ local: "9090", remote: "" });
    expect(startButton().disabled).toBe(true);
  });

  // A dead row is HISTORY, not a claim on the port. Core keeps a forward that
  // gave up on screen deliberately (a tunnel that died underneath the reader is
  // news), and dismissing it is a separate gesture — so counting it here made
  // the reader clear the history of the failure before they could retry it,
  // which is the one thing they came back to the dialog to do.
  it("lets a port that a failed forward gave up on be forwarded again", async () => {
    store.list = [dead(8080)];
    open();
    await fillIn({ local: "8080" });

    expect(screen.queryByText(/already forwarded/)).toBeNull();
    expect(startButton().disabled).toBe(false);
    // Not merely enabled: the retry actually goes out, on the same port.
    await userEvent.click(startButton());
    await waitFor(() => expect(core.startPortForward).toHaveBeenCalledTimes(1));
    expect(core.startPortForward).toHaveBeenCalledWith(
      expect.objectContaining({ localPort: 8080 }),
    );
  });

  // The boundary `isForwardEnded` draws, asserted from this side too: only
  // `failed` frees the port. A tunnel that is reconnecting kept its listener,
  // so its number is still taken and §A.4's error still stands.
  it("still refuses a port a reconnecting forward is holding", async () => {
    store.list = [flapping(8080)];
    open();
    await fillIn({ local: "8080" });

    expect(screen.getByText("Port 8080 is already forwarded.")).toBeTruthy();
    expect(startButton().disabled).toBe(true);
    expect(core.startPortForward).not.toHaveBeenCalled();
  });
});

describe("NewForwardDialog — the equivalent command", () => {
  it("is core's command for the fields as they stand", async () => {
    open();
    await fillIn({ local: "9090", remote: "8080" });
    const expected = toKubectl({
      action: "port-forward",
      kind: "Service",
      name: "checkout-api",
      context: CONTEXT,
      namespace: "checkout",
      localPort: 9090,
      remotePort: 8080,
    });
    expect(screen.getByText(expected)).toBeTruthy();
    // And it really is the port-forward command, so the line above is not two
    // identical mistakes agreeing.
    expect(expected).toContain("port-forward svc/checkout-api 9090:8080");
  });

  it("follows the fields as they change", async () => {
    open();
    await fillIn({ local: "9090", remote: "8080" });
    expect(screen.getByText(/9090:8080/)).toBeTruthy();
    await userEvent.clear(field("Remote port"));
    await userEvent.type(field("Remote port"), "443");
    await waitFor(() => expect(screen.getByText(/9090:443/)).toBeTruthy());
    expect(screen.queryByText(/9090:8080/)).toBeNull();
    await userEvent.selectOptions(field("Target"), "pod/checkout-api-5c8b7f2d9-mk3wl");
    await waitFor(() =>
      expect(screen.getByText(/port-forward pod\/checkout-api-5c8b7f2d9-mk3wl 9090:443/)).toBeTruthy(),
    );
  });
});

describe("NewForwardDialog — starting the forward", () => {
  it("asks core for exactly the forward the fields describe", async () => {
    open();
    await fillIn({ local: "9090", remote: "8080", target: "pod/checkout-api-5c8b7f2d9-mk3wl" });
    await userEvent.click(startButton());
    await waitFor(() => expect(core.startPortForward).toHaveBeenCalledTimes(1));
    expect(core.startPortForward).toHaveBeenCalledWith({
      context: CONTEXT,
      namespace: "checkout",
      // The KIND, not the `pod/` prefix the select shows.
      kind: "Pod",
      name: "checkout-api-5c8b7f2d9-mk3wl",
      localPort: 9090,
      remotePort: 8080,
    });
  });

  it("closes once the tunnel is up", async () => {
    const onClose = open();
    await fillIn();
    await userEvent.click(startButton());
    await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
  });

  it("says where the forward is, on the desktop, using the port the BACKEND bound", async () => {
    // The backend may not get the port that was asked for. The toast reports
    // what came back, never what was typed.
    core.startPortForward.mockResolvedValue({ id: 7, localPort: 9091 });
    open();
    await fillIn({ local: "9090" });
    await userEvent.click(startButton());
    await waitFor(() => expect(core.notify.success).toHaveBeenCalledTimes(1));
    expect(core.notify.success).toHaveBeenCalledWith(
      "Forwarding localhost:9091 to svc/checkout-api",
    );
  });

  it("says the proxy address in the browser, where a container's loopback is unreachable", async () => {
    platform.isTauri.mockReturnValue(false);
    open();
    await fillIn({ local: "9090" });
    await userEvent.click(startButton());
    await waitFor(() => expect(core.notify.success).toHaveBeenCalledTimes(1));
    const said = core.notify.success.mock.calls[0][0] as string;
    expect(said).toBe(`Forwarding ${window.location.origin}/pf/7/ to svc/checkout-api`);
    // Said a second way on purpose: jsdom's own origin contains "localhost",
    // so the equality above would still hold for a dialog that had hardcoded
    // §A.4's desktop answer at some other port. THIS is the property.
    expect(said).toContain("/pf/7/");
    expect(said).not.toContain("localhost:9090");
  });

  it("reports a refused start in words, and stays open", async () => {
    const onClose = vi.fn();
    core.startPortForward.mockRejectedValue(
      new Error("ApiError: Unauthorized (Status { metadata: Some(ListMeta { .. }) })"),
    );
    open(onClose);
    await fillIn();
    await userEvent.click(startButton());
    expect(await screen.findByText(/rejected your credentials/i)).toBeTruthy();
    // `describeError`'s classification, not the struct — and the struct only
    // inside the disclosure.
    const alert = screen.getByText(/rejected your credentials/i).closest("[data-tone]");
    expect(alert?.querySelector('[data-slot="raw"]')?.textContent).toContain("ApiError");
    expect(screen.getByText(/Could not forward svc\/checkout-api/i)).toBeTruthy();
    expect(onClose).not.toHaveBeenCalled();
    expect(core.notify.success).not.toHaveBeenCalled();
  });
});

describe("NewForwardDialog — the browser switch", () => {
  it("opens nothing when it is off", async () => {
    open();
    await fillIn();
    await userEvent.click(startButton());
    await waitFor(() => expect(core.notify.success).toHaveBeenCalled());
    expect(core.openExternal).not.toHaveBeenCalled();
    expect(opened).not.toHaveBeenCalled();
  });

  it("opens the desktop's loopback when it is on — through core, never window.open", async () => {
    core.startPortForward.mockResolvedValue({ id: 7, localPort: 9091 });
    open();
    await fillIn({ local: "9090" });
    await userEvent.click(screen.getByRole("switch", { name: "Open in browser when it comes up" }));
    await userEvent.click(startButton());
    await waitFor(() => expect(core.openExternal).toHaveBeenCalledTimes(1));
    // With the scheme: `forwardAddress` answers a bare authority here, and a
    // bare `localhost:9091` is not a URL — it resolves against the page.
    expect(core.openExternal.mock.calls[0][0]).toBe("http://localhost:9091");
    // The switch used to call this directly, which does nothing at all in a
    // Tauri WebView and does it without an error (#348).
    expect(opened).not.toHaveBeenCalled();
  });

  it("opens the SAME address it promised in web mode, not a hardcoded localhost", async () => {
    platform.isTauri.mockReturnValue(false);
    open();
    await fillIn({ local: "9090" });
    await userEvent.click(screen.getByRole("switch", { name: "Open in browser when it comes up" }));
    await userEvent.click(startButton());
    await waitFor(() => expect(core.openExternal).toHaveBeenCalledTimes(1));
    const url = core.openExternal.mock.calls[0][0] as string;
    expect(url).toBe(`${window.location.origin}/pf/7/`);
    // Said twice: jsdom's own origin contains "localhost", so the equality
    // above would still hold for a dialog that had hardcoded the desktop
    // answer at some other port. This is the property.
    expect(url).toContain("/pf/7/");
    expect(url).not.toContain("localhost:9090");
  });

  it("says the browser could not be opened without losing the forward that started", async () => {
    // The tunnel is up either way. Turning a browser that would not open into
    // a banner over a dialog that is closing would report the wrong failure.
    core.openExternal.mockRejectedValue(new Error("handler error: no default browser"));
    const onClose = open();
    await fillIn({ local: "9090" });
    await userEvent.click(screen.getByRole("switch", { name: "Open in browser when it comes up" }));
    await userEvent.click(startButton());
    await waitFor(() => expect(core.notify.error).toHaveBeenCalledTimes(1));
    const [title, detail] = core.notify.error.mock.calls[0] as [string, string];
    expect(title).toMatch(/browser/i);
    // `describeError`'s wording, and the internal prefix stripped off it.
    expect(detail).toContain("no default browser");
    expect(detail).not.toContain("handler error:");
    // The forward itself still succeeded, and the dialog still got out of the
    // way.
    expect(core.notify.success).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
  });

  it("hints the desktop address §A.4 writes, for the port in the field", async () => {
    open();
    await fillIn({ local: "9090" });
    const hint = document.getElementById(
      screen
        .getByRole("switch", { name: "Open in browser when it comes up" })
        .getAttribute("aria-describedby") ?? "",
    );
    expect(hint?.textContent).toBe("http://localhost:9090");
  });

  it("hints the proxy's shape in web mode, without inventing an id", async () => {
    platform.isTauri.mockReturnValue(false);
    open();
    await fillIn({ local: "9090" });
    const hint = document.getElementById(
      screen
        .getByRole("switch", { name: "Open in browser when it comes up" })
        .getAttribute("aria-describedby") ?? "",
    );
    expect(hint?.textContent).toBe(`${window.location.origin}/pf/<id>/`);
    // Not the desktop hint, and not a made-up number where the id goes.
    expect(hint?.textContent).not.toContain("localhost:9090");
    expect(hint?.textContent).not.toContain("/pf/-");
  });
});

describe("NewForwardDialog — what it must not leak", () => {
  it("puts no address, port or command in a title attribute", async () => {
    open();
    await fillIn({ local: "9090", remote: "8080" });
    const joined = Array.from(document.querySelectorAll("[title]"))
      .map((el) => el.getAttribute("title") ?? "")
      .join("\n");
    expect(joined).not.toContain("localhost");
    expect(joined).not.toContain("/pf/");
    expect(joined).not.toContain("port-forward");
    expect(joined).not.toContain("--context");
    for (const port of ["9090", "8080"]) expect(joined).not.toContain(port);
  });
});

/**
 * The dialog reached from a Service's Ports row, a container's port chip or
 * the row menu's `Port forward` — the three doors that used to be dead ends.
 *
 * What a prefill must be: a STARTING POINT. The target and both ports stay
 * editable, and the local port stays EMPTY — the OS picks a free one, and
 * seeding it with the remote port is how forwarding a port you already hold
 * locally walks straight into §A.4's clash error on the commonest case there
 * is.
 */
describe("NewForwardDialog — opened from the thing being forwarded", () => {
  /**
   * §A.4's own fixture lists this pod LAST, and 9376 is neither the Remote
   * port field's placeholder (8080) nor any port the fixture mentions — so a
   * dialog that merely defaulted to its first option, or left its fields
   * alone, cannot pass the assertions below by accident.
   */
  const POD_TARGET = { kind: "Pod", name: "checkout-api-5c8b7f2d9-mk3wl", remotePort: 9376 };

  function openOn(
    target: { kind: string; name: string; remotePort?: number },
    onClose = vi.fn(),
  ) {
    render(
      <NewForwardDialog context={CONTEXT} namespace="checkout" target={target} onClose={onClose} />,
    );
    return onClose;
  }

  const select = (name: string) => field(name) as HTMLSelectElement;
  const input = (name: string) => field(name) as HTMLInputElement;

  it("starts on the target it was handed, named the way its KIND is named", async () => {
    openOn(POD_TARGET);
    await screen.findByRole("dialog");
    await waitFor(() => expect(select("Target").value).toBe("pod/checkout-api-5c8b7f2d9-mk3wl"));
    // `pod/`, from `kindToForwardTarget` — not `svc/`, which is what a prefill
    // that ignored the kind and guessed would produce.
    expect(select("Target").value.startsWith("pod/")).toBe(true);
    await waitFor(() => expect(select("Namespace").value).toBe("checkout"));
  });

  it("prefills the remote port, and offers a local one that is not it", async () => {
    // Drawn at random from a range nothing claims by convention, rather than
    // mirroring the far end — a port worth forwarding is one this machine
    // plausibly already serves, and only srelens' own forwards can be seen
    // from here.
    vi.spyOn(Math, "random").mockReturnValue(0);
    openOn(POD_TARGET);
    await screen.findByRole("dialog");
    expect(input("Remote port").value).toBe("9376");
    // The values came from the click and the draw, not from the fields: both
    // placeholders say something else entirely.
    expect(input("Remote port").placeholder).toBe("8080");
    expect(input("Local port").placeholder).toBe("9090");
    expect(input("Local port").value).toBe(String(OFFER_LOW));
    expect(input("Local port").value).not.toBe(input("Remote port").value);
  });

  it("lets a cleared local port mean 'any free one', which is what it promised", async () => {
    // The offer is an offer. Clearing it asks the backend to choose, which is
    // the only answer that cannot collide with something outside srelens —
    // and it was the documented fallback while being impossible to reach:
    // `portOf` returns null for blank AND for invalid, so a cleared field
    // disabled Start exactly as "abc" did.
    vi.spyOn(Math, "random").mockReturnValue(0);
    openOn(POD_TARGET);
    await screen.findByRole("dialog");
    await userEvent.clear(field("Local port"));

    // kubectl's own spelling for a random local port.
    await waitFor(() =>
      expect(screen.getByText(/port-forward pod\/checkout-api-5c8b7f2d9-mk3wl :9376$/)).toBeTruthy(),
    );
    await waitFor(() => expect(startButton().disabled).toBe(false));
    await userEvent.click(startButton());
    await waitFor(() => expect(core.startPortForward).toHaveBeenCalledTimes(1));
    // No `localPort` key at all — not `undefined`, not 0.
    expect(core.startPortForward.mock.calls[0][0]).not.toHaveProperty("localPort");
  });

  it("still refuses a number that is not a port", async () => {
    // Blank and invalid must not collapse back together: an empty field is a
    // decision, 99999 is a mistake. The field is `type="number"`, so letters
    // cannot be entered at all — out of range is the invalid case that is
    // actually reachable.
    openOn(POD_TARGET);
    await screen.findByRole("dialog");
    await userEvent.clear(field("Local port"));
    await userEvent.type(field("Local port"), "99999");
    await waitFor(() => expect(startButton().disabled).toBe(true));
  });

  it("draws only from the offer range", () => {
    // The unit, so the range is pinned without a component in the way. Below
    // 49152 the OS is not handing the same numbers out for outbound sockets;
    // above 1023 nothing well-known is being trodden on.
    expect(offerLocalPort([], () => 0)).toBe(OFFER_LOW);
    expect(offerLocalPort([], () => 0.9999999)).toBe(OFFER_HIGH);
  });

  it("passes over a port srelens is already forwarding, and wraps", () => {
    const held = [holding(OFFER_LOW), holding(OFFER_LOW + 1)] as ActiveForward[];
    expect(offerLocalPort(held, () => 0)).toBe(OFFER_LOW + 2);
    // Drawn at the very top with the top held, it comes round rather than
    // running off the end of the range.
    expect(offerLocalPort([holding(OFFER_HIGH)] as ActiveForward[], () => 0.9999999)).toBe(OFFER_LOW);
  });

  it("asks only for what is missing, not for the target it was handed", async () => {
    // Opened from a port's Forward, everything but the local port arrives
    // filled. A line reading "choose a target and both ports" then asks the
    // reader for two things they already have, and it is the first thing they
    // read under the command they came to copy.
    // Arriving from a port, nothing is missing — the command is there to copy
    // rather than a sentence asking for fields the reader already handed over.
    openOn(POD_TARGET);
    await screen.findByRole("dialog");
    expect(screen.queryByText(/Fill in/)).toBeNull();

    // Clearing the LOCAL port is not a gap — it asks for any free one. The
    // remote port has no such fallback, so that is the one that can go
    // missing, and it is named alone.
    await userEvent.clear(field("Remote port"));
    expect(await screen.findByText("Fill in a remote port to see it.")).toBeTruthy();
    expect(screen.queryByText(/a target/)).toBeNull();
  });

  it("names the namespace when that is the field the command still wants", async () => {
    // A door that hands over a target but no namespace: the target, both ports
    // and `localUsable` are all satisfied, and the one thing missing is the
    // namespace — which was left out of the list entirely, so the join came out
    // empty and the line read "Fill in  to see it."
    render(<NewForwardDialog context={CONTEXT} target={POD_TARGET} onClose={vi.fn()} />);
    await screen.findByRole("dialog");

    expect(screen.getByText("Fill in a namespace to see it.")).toBeTruthy();
    expect(screen.queryByText("Fill in  to see it.")).toBeNull();
  });

  it("says the listing is still out rather than asking for fields that are filled", async () => {
    // The whole duration of a namespace change's round trip: the target the
    // reader arrived with is not in the new namespace's listing yet, so
    // `chosen` is undefined while every field the old list checked is filled.
    // "Fill in  to see it." was what that state printed.
    core.listServices.mockReturnValue(new Promise(() => {}));
    core.listPods.mockReturnValue(new Promise(() => {}));
    openOn(POD_TARGET);
    await screen.findByRole("dialog");
    await waitFor(() => expect(core.listNamespaces).toHaveBeenCalled());

    await userEvent.selectOptions(field("Namespace"), "payments");

    expect(screen.queryByText("Fill in  to see it.")).toBeNull();
    expect(screen.getByText("srelens is still listing what payments can forward.")).toBeTruthy();
  });

  it("prefills nothing when there is no target — the screen's own way in, unchanged", async () => {
    open();
    await screen.findByRole("dialog");
    expect(select("Target").value).toBe("");
    expect(input("Remote port").value).toBe("");
  });

  it("writes a Service's command with svc/, a Pod's with pod/", async () => {
    openOn({ kind: "Service", name: "checkout-api", remotePort: 9376 });
    await screen.findByRole("dialog");
    // Cleared first: the field arrives with the offer in it, so typing alone
    // would append to it.
    await userEvent.clear(field("Local port"));
    await userEvent.type(field("Local port"), "9091");
    const expected = toKubectl({
      action: "port-forward",
      kind: "Service",
      name: "checkout-api",
      context: CONTEXT,
      namespace: "checkout",
      localPort: 9091,
      remotePort: 9376,
    });
    await waitFor(() => expect(screen.getByText(expected)).toBeTruthy());
    expect(expected).toContain("port-forward svc/checkout-api 9091:9376");
  });

  it("is a starting point, not a lock: target and both ports stay editable", async () => {
    openOn(POD_TARGET);
    await screen.findByRole("dialog");
    // The prefill is really there first — without this the rest of the test
    // is just the dialog being filled in by hand, which it always allowed.
    await waitFor(() => expect(select("Target").value).toBe("pod/checkout-api-5c8b7f2d9-mk3wl"));
    expect(input("Remote port").value).toBe("9376");
    await waitFor(() =>
      expect(within(field("Target")).queryByRole("option", { name: "svc/checkout-web" })).toBeTruthy(),
    );
    await userEvent.selectOptions(field("Target"), "svc/checkout-web");
    await userEvent.clear(field("Remote port"));
    await userEvent.type(field("Remote port"), "443");
    await userEvent.clear(field("Local port"));
    await userEvent.type(field("Local port"), "9091");
    await waitFor(() =>
      expect(screen.getByText(/port-forward svc\/checkout-web 9091:443/)).toBeTruthy(),
    );
  });

  it("keeps offering the handed target when the listing that would name it refuses", async () => {
    // The reader got here by clicking the thing itself; a Pods listing that
    // came back forbidden is not a reason to forget what they clicked.
    core.listPods.mockResolvedValue({ error: "ApiError: forbidden" });
    openOn(POD_TARGET);
    await screen.findByRole("dialog");
    await waitFor(() => expect(core.listPods).toHaveBeenCalled());
    await waitFor(() => expect(select("Target").value).toBe("pod/checkout-api-5c8b7f2d9-mk3wl"));
  });

  it("starts exactly the forward the reader arrived with, without touching a field", async () => {
    vi.spyOn(Math, "random").mockReturnValue(0);
    openOn(POD_TARGET);
    await screen.findByRole("dialog");
    // Nothing typed: the offer is enough to start on.
    await waitFor(() => expect(startButton().disabled).toBe(false));
    await userEvent.click(startButton());
    await waitFor(() => expect(core.startPortForward).toHaveBeenCalledTimes(1));
    expect(core.startPortForward).toHaveBeenCalledWith({
      context: CONTEXT,
      namespace: "checkout",
      kind: "Pod",
      name: "checkout-api-5c8b7f2d9-mk3wl",
      localPort: OFFER_LOW,
      remotePort: 9376,
    });
  });

  it("drops the prefilled target when the reader moves to another namespace", async () => {
    openOn(POD_TARGET);
    await screen.findByRole("dialog");
    await waitFor(() => expect(select("Target").value).toBe("pod/checkout-api-5c8b7f2d9-mk3wl"));
    core.listServices.mockResolvedValue({ services: [] });
    core.listPods.mockResolvedValue({ pods: [] });
    await userEvent.selectOptions(field("Namespace"), "payments");
    await waitFor(() => expect(core.listPods).toHaveBeenCalledWith(CONTEXT, "payments"));
    await waitFor(() => expect(select("Target").value).toBe(""));
  });
});
