import { useEffect, useMemo, useState, useSyncExternalStore } from "react";
import {
  type ActiveForward,
  browsable,
  describeError,
  forwardAddress,
  getForwards,
  kindToForwardTarget,
  listNamespaces,
  listPods,
  listServices,
  notify,
  openExternal,
  startPortForward,
  subscribeForwards,
  toKubectl,
} from "@srelens/core";
import {
  Button,
  CopyCommand,
  Dialog,
  Field,
  Select,
  SubHead,
  Switch,
  TextInput,
} from "@srelens/ui-kit";
import { FailureAlert } from "../../lib/errorCopy";

/** §A.4's width, in px. */
const WIDTH = 480;

/** The highest port a TCP socket can bind. */
const MAX_PORT = 65_535;

/**
 * The id handed to `forwardAddress` to ask *where would a forward on this port
 * be reachable from*, before there is a forward to ask about.
 *
 * Negative because no forward can ever carry it, so the answer it produces is
 * unmistakably a template rather than a real address that happens to be wrong.
 * See {@link plannedAddress}.
 */
const PLACEHOLDER_ID = -1;

/** One thing in the namespace that a forward can be pointed at. */
interface Target {
  /** kubectl's own name for it — `svc/checkout-api` — and the select's value. */
  value: string;
  /** The Kubernetes kind, which is what `startPortForward` and `toKubectl` take. */
  kind: string;
  name: string;
}

/** The port a field holds, or null when it holds nothing a socket could bind. */
function portOf(text: string): number | null {
  const trimmed = text.trim();
  if (!trimmed) return null;
  const n = Number(trimmed);
  return Number.isInteger(n) && n > 0 && n <= MAX_PORT ? n : null;
}

/**
 * Where a forward on this local port WILL be reachable from, in the form the
 * switch's hint shows and the switch itself will open.
 *
 * §A.4 writes the hint as a literal `http://localhost:{local}`, which is the
 * desktop answer and only the desktop answer — the same defect as §13's Copy
 * URL. `forwardAddress` is the one place that decides between the two
 * platforms, and it needs an id that does not exist until the start returns,
 * so it is asked with {@link PLACEHOLDER_ID} and the stand-in is put back as
 * `<id>`. On the desktop the id never appears in the answer and the hint is
 * exact — §A.4's line, verbatim. In web mode the reader gets the proxy's real
 * shape instead of a loopback their browser cannot reach or an id nothing has
 * assigned yet. Either way the hint and the toast come from ONE rule.
 */
function plannedAddress(localPort: number): string {
  const address = forwardAddress({ id: PLACEHOLDER_ID, localPort });
  return browsable(address.replace(`/pf/${PLACEHOLDER_ID}/`, "/pf/<id>/"));
}

/**
 * The thing a reader was already looking at when they asked to forward it —
 * a Service's Ports row, a container's port, a row in a list.
 *
 * `kind` is the KUBERNETES kind (`Service`, `Pod`), never kubectl's short
 * form: `kindToForwardTarget` owns that mapping, and it is what decides
 * whether the equivalent command reads `svc/` or `pod/`.
 *
 * `remotePort` is the port on the far end — the one that was clicked. There
 * is deliberately no local port here: see {@link NewForwardDialog}.
 */
export interface ForwardTarget {
  kind: string;
  name: string;
  remotePort?: number;
}

export interface NewForwardDialogProps {
  /** The cluster the forward is made in — a kubeconfig context NAME. */
  context: string;
  /** Where the namespace select starts, when the caller knows a good answer. */
  namespace?: string;
  /**
   * What the reader arrived from, when they came through one of the doors
   * that already knows the answer. Absent from `/forwards`' own header
   * action, which starts on nothing.
   */
  target?: ForwardTarget;
  /** Cancel, escape, the header's control, and a forward that came up. */
  onClose: () => void;
}

/**
 * §A.4's `New port forward` — the one way to start a tunnel.
 *
 * **The clash check runs here, not on the backend.** §A.4's error is
 * `Port <n> is already forwarded.` and core's forwards store already knows
 * every live `localPort`, so the answer is on this side of the wire: the field
 * says so as the digits are typed and `Start forward` is dead while it holds.
 * Letting the request go out to be refused would be a slower way to learn the
 * same thing, and the sentence that came back would be about a bind error
 * rather than about the other tunnel. The store is subscribed to rather than
 * read once, so stopping the forward that holds the port frees it here without
 * the reader retyping anything.
 *
 * **The address is never assembled here.** §A.4's `http://localhost:{local}`
 * hint and its `Forwarding localhost:<n> to <target>` toast are both written
 * against the desktop, where the tunnel really is on this machine's loopback;
 * in web mode srelens runs in a container whose loopback the browser cannot
 * reach and the forward is served from a same-origin `/pf/<id>/` proxy.
 * `forwardAddress` decides that, and the hint, the toast and the browser
 * switch all read it — so the switch opens exactly the address the hint
 * promised. The toast can only ask after the start returns, because that is
 * when the id exists; the hint asks with a placeholder id
 * ({@link plannedAddress}).
 *
 * The target list is the namespace's Services and Pods, named the way kubectl
 * names them through core's `kindToForwardTarget` — the same function the
 * equivalent command and the forwards table's Target cell go through. The
 * `kind` travels beside the label rather than being parsed back out of it.
 *
 * `listNamespaces`, `listServices` and `listPods` all report failure by
 * RETURNING an error rather than throwing, so nothing here wraps them in a
 * try/catch and calls that error handling; the field is read. `startPortForward`
 * does throw, and that one is caught.
 *
 * **A `target` is a starting point, not a lock.** Every door that opens this
 * with one — a Service's Ports row, a container's port, the row menu's
 * `Port forward` — is handing over what the reader was looking at, and they
 * may well want a different one; the target select and both port fields stay
 * exactly as editable as they are from the header action.
 *
 * **The local port is offered, not demanded** — and it is a free port drawn
 * at random rather than the remote one again. See {@link offerLocalPort}: a
 * port worth forwarding is one this machine plausibly already serves, and
 * srelens can only see its own forwards. Clearing the field sends no local
 * port at all and lets the OS choose, which is the answer that cannot
 * collide.
 */
/**
 * The low and high ends of the range a suggested local port is drawn from.
 *
 * Above the well-known (0-1023) and the bulk of the registered service ports,
 * so a suggestion does not land on something a developer runs by habit; and
 * below 49152, where the OS assigns its own ephemeral ports for outbound
 * connections, so a listener bound here is not competing with the kernel for
 * the same number.
 */
export const OFFER_LOW = 10000;
export const OFFER_HIGH = 32767;

/**
 * A local port to offer: a free one at random, not the remote port again.
 *
 * Mirroring the far end reads well — `50051:50051` — and is the more likely
 * of the two to fail, because a port worth forwarding is a port something on
 * this machine plausibly already serves. Only srelens' own forwards can be
 * checked from here; anything else holding the number is invisible until the
 * start fails. A number drawn from a range nothing claims by convention is
 * far likelier to be free on the first try.
 *
 * `random` is a parameter so a test can pin the draw. Clearing the field
 * sends no local port at all and lets the OS choose, which remains the answer
 * that cannot collide.
 */
export function offerLocalPort(
  taken: readonly ActiveForward[],
  random: () => number = Math.random,
): number {
  const held = new Set(taken.map((f) => f.localPort));
  const span = OFFER_HIGH - OFFER_LOW + 1;
  const first = OFFER_LOW + Math.floor(random() * span);
  // Walk on from the draw rather than re-drawing, so this terminates even if
  // the random source is degenerate, and wrap so the whole range is reachable.
  for (let i = 0; i < span; i += 1) {
    const port = OFFER_LOW + ((first - OFFER_LOW + i) % span);
    if (!held.has(port)) return port;
  }
  return first;
}

export function NewForwardDialog({
  context,
  namespace: initial,
  target: arrivedFrom,
  onClose,
}: NewForwardDialogProps) {
  const forwards = useSyncExternalStore(subscribeForwards, getForwards, getForwards);

  /**
   * The handed-over target as an OPTION — the same shape the listings produce,
   * so nothing downstream has to know which of the two it came from.
   *
   * Memoised on the primitives rather than on the prop object, because every
   * door passes an object literal that is new on each of its own renders.
   */
  const prefilled = useMemo<Target | null>(
    () =>
      arrivedFrom
        ? {
            value: `${kindToForwardTarget(arrivedFrom.kind)}/${arrivedFrom.name}`,
            kind: arrivedFrom.kind,
            name: arrivedFrom.name,
          }
        : null,
    [arrivedFrom?.kind, arrivedFrom?.name],
  );
  /** The namespace a prefilled target belongs to — the one it was opened on. */
  const homeNamespace = initial ?? "";

  const [namespaces, setNamespaces] = useState<string[] | null>(null);
  const [namespace, setNamespace] = useState(homeNamespace);
  const [targets, setTargets] = useState<Target[] | null>(null);
  const [target, setTarget] = useState(prefilled?.value ?? "");
  const [localText, setLocalText] = useState(() =>
    arrivedFrom?.remotePort == null
      ? ""
      : String(offerLocalPort(getForwards())),
  );
  const [remoteText, setRemoteText] = useState(
    arrivedFrom?.remotePort == null ? "" : String(arrivedFrom.remotePort),
  );
  const [inBrowser, setInBrowser] = useState(false);
  const [busy, setBusy] = useState(false);

  /**
   * The last thing that went wrong, whichever it was: a listing that came back
   * with an error field, or a start that threw. One slot rather than three —
   * the dialog is small enough that a second banner would push the footer off
   * a short window, and the reader only ever acts on the most recent one.
   */
  const [failure, setFailure] = useState<{ title: string; error: unknown } | null>(null);

  /**
   * The one target value a listing is not allowed to clear — the prefilled
   * one, and only while the reader is still in the namespace it came from.
   * `""` (which no option ever holds) when there is nothing to protect.
   */
  const keptOnArrival = prefilled && namespace === homeNamespace ? prefilled.value : "";

  useEffect(() => {
    let live = true;
    setNamespaces(null);
    // A forward is made in ONE cluster, and the rail's selection is the only
    // answer the app has to which. With no cluster in focus there is nothing
    // to list, and `listNamespaces("")` would go to the backend to be told so;
    // the empty options say it here, in the control that would have offered
    // them. See the Namespace placeholder below.
    if (!context) {
      setNamespaces([]);
      return;
    }
    void listNamespaces(context).then((outcome) => {
      if (!live) return;
      if (outcome.error) {
        setFailure({ title: "Could not list this cluster's namespaces", error: outcome.error });
        setNamespaces([]);
        return;
      }
      setNamespaces(outcome.namespaces ?? []);
    });
    return () => {
      live = false;
    };
  }, [context]);

  useEffect(() => {
    if (!namespace) {
      setTargets(null);
      return;
    }
    let live = true;
    setTargets(null);
    void Promise.all([listServices(context, namespace), listPods(context, namespace)]).then(
      ([services, pods]) => {
        if (!live) return;
        const error = services.error || pods.error;
        if (error) {
          setFailure({ title: `Could not list what ${namespace} can forward`, error });
        }
        // Whatever answered is still worth offering: a cluster that lists its
        // Services but refuses its Pods can still forward a Service.
        const listed: Target[] = [
          ...(services.services ?? []).map((s) => ({
            value: `${kindToForwardTarget("Service")}/${s.name}`,
            kind: "Service",
            name: s.name,
          })),
          ...(pods.pods ?? []).map((p) => ({
            value: `${kindToForwardTarget("Pod")}/${p.name}`,
            kind: "Pod",
            name: p.name,
          })),
        ];
        setTargets(listed);
        // The selection is reconciled against what this namespace actually
        // has, rather than cleared the moment the namespace changes: clearing
        // up front would wipe a prefilled target before its own listing had a
        // chance to confirm it. A target the reader ARRIVED with survives a
        // listing that never named it (see `offered`) — but only in its own
        // namespace.
        setTarget((current) =>
          current === keptOnArrival || listed.some((t) => t.value === current) ? current : "",
        );
      },
    );
    return () => {
      live = false;
    };
  }, [context, namespace, keptOnArrival]);

  /**
   * Everything the target select offers: what the namespace listed, plus the
   * target the reader arrived from when that listing did not name it.
   *
   * A Pods listing that came back forbidden — or has simply not landed yet —
   * is not a reason to forget what was clicked. The reader got here from the
   * thing itself, so it is a target whether or not a list agrees.
   */
  const offered = useMemo<Target[]>(() => {
    const listed = targets ?? [];
    if (!prefilled || namespace !== homeNamespace) return listed;
    return listed.some((t) => t.value === prefilled.value) ? listed : [prefilled, ...listed];
  }, [targets, prefilled, namespace, homeNamespace]);

  // Gated on the namespace as well as the value: a prefilled target is offered
  // before any listing lands, and a forward with no namespace is not a forward.
  const chosen = useMemo(
    () => (namespace ? offered.find((t) => t.value === target) : undefined),
    [offered, target, namespace],
  );
  const localPort = portOf(localText);
  const remotePort = portOf(remoteText);
  /**
   * Blank is a DECISION, not a mistake — it asks the backend for any free
   * port, which is the one answer that cannot collide with something outside
   * srelens. `portOf` cannot tell the two apart: it answers null for an empty
   * field and for "abc" alike, which is how the documented fallback ended up
   * unreachable.
   */
  const localBlank = localText.trim() === "";
  const localUsable = localBlank || localPort !== null;

  /** §A.4's one field error, decided against the live store. */
  const clash = localPort !== null && forwards.some((f) => f.localPort === localPort);

  /** The fields the equivalent command still wants, in the order they are read. */
  const missingForCommand = [
    target ? null : "a target",
    localUsable ? null : "a local port",
    remotePort === null ? "a remote port" : null,
  ]
    .filter((f): f is string => f !== null)
    .join(" and ");

  const command =
    chosen && localUsable && remotePort !== null
      ? toKubectl({
          action: "port-forward",
          kind: chosen.kind,
          name: chosen.name,
          context,
          namespace,
          ...(localPort === null ? {} : { localPort }),
          remotePort,
        })
      : null;

  const ready = command !== null && !clash && !busy;

  async function start() {
    if (!chosen || !localUsable || remotePort === null || clash) return;
    setFailure(null);
    setBusy(true);
    try {
      const started = await startPortForward({
        context,
        namespace,
        kind: chosen.kind,
        name: chosen.name,
        // Omitted entirely when the field is blank: `ForwardRequest.localPort`
        // is optional and the backend binds a free port when it is absent.
        ...(localPort === null ? {} : { localPort }),
        remotePort,
      });
      // The port the BACKEND bound, not the one that was typed: the two differ
      // whenever the request could not have the port it asked for.
      const address = forwardAddress({ id: started.id, localPort: started.localPort });
      notify.success(`Forwarding ${address} to ${chosen.value}`);
      // Core's opener, not `window.open`: inside a Tauri WebView that call
      // returns without opening anything and without failing, so the switch
      // read as wired up and did nothing (#348). A browser that will not open
      // is a TOAST rather than the banner above — the tunnel is up either way,
      // and the dialog it would be drawn in is already closing.
      if (inBrowser) {
        try {
          await openExternal(browsable(address));
        } catch (e) {
          notify.error("Couldn't open the forward in your browser", describeError(e).detail);
        }
      }
      onClose();
    } catch (e) {
      setFailure({ title: `Could not forward ${chosen.value}`, error: e });
      setBusy(false);
    }
  }

  const namespaceOptions = (namespaces ?? []).map((n) => ({ value: n }));
  const targetOptions = offered.map((t) => ({ value: t.value }));

  return (
    <Dialog
      title="New port forward"
      maxWidth={WIDTH}
      onClose={onClose}
      footer={
        <>
          <Button variant="secondary" size="sm" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" size="sm" disabled={!ready} onClick={() => void start()}>
            Start forward
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-3 p-3">
        {failure && <FailureAlert tone="sev" title={failure.title} error={failure.error} />}

        {/* §A.4's two-column grid, in §A.4's order. Target and Namespace share
            a row, so the target select sitting first costs nothing: both are on
            screen at once and the target one says what it still needs. */}
        <div className="grid grid-cols-2 gap-x-3">
          <Field label="Target">
            <Select
              value={target}
              onValueChange={setTarget}
              options={targetOptions}
              className="w-full"
              // A prefilled target is offered before any listing lands, so
              // the control it is showing must not be dead in the meantime.
              disabled={!namespace || (targets === null && targetOptions.length === 0)}
              placeholder={
                !namespace
                  ? "Pick a namespace first"
                  : targets === null
                    ? "Loading…"
                    : targetOptions.length === 0
                      ? "Nothing here to forward"
                      : "Choose a target"
              }
            />
          </Field>
          <Field label="Namespace">
            <Select
              value={namespace}
              onValueChange={setNamespace}
              options={namespaceOptions}
              className="w-full"
              disabled={namespaces === null || !context}
              placeholder={
                !context
                  ? "Pick a cluster in the rail first"
                  : namespaces === null
                    ? "Loading…"
                    : "Choose a namespace"
              }
            />
          </Field>
          <Field
            label="Local port"
            // §A.4's wording, exactly. The banner slot above is for failures
            // that came from somewhere else; this one belongs to the field.
            error={clash ? `Port ${localPort} is already forwarded.` : undefined}
          >
            <TextInput
              value={localText}
              onValueChange={setLocalText}
              type="number"
              invalid={clash}
              placeholder="9090"
            />
          </Field>
          <Field label="Remote port">
            <TextInput
              value={remoteText}
              onValueChange={setRemoteText}
              type="number"
              placeholder="8080"
            />
          </Field>
        </div>

        <Switch
          on={inBrowser}
          onChange={setInBrowser}
          label="Open in browser when it comes up"
          hint={
            localBlank
              ? "srelens picks a free port, and opens it when it comes up."
              : localPort === null
                ? "Name a local port and this is where it opens."
                : plannedAddress(localPort)
          }
        />

        <div>
          <SubHead variant="caps">Equivalent command</SubHead>
          <div className="mt-1">
            {command ? (
              <CopyCommand command={command} />
            ) : (
              /* Names what is actually missing, rather than listing every
                 field. Opened from a port's `Forward` the target, namespace
                 and remote port all arrive filled, and a line reading "choose
                 a target" then asks the reader for the one thing they have
                 already done. */
              <span className="text-[0.75rem] text-muted">{`Fill in ${missingForCommand} to see it.`}</span>
            )}
          </div>
        </div>
      </div>
    </Dialog>
  );
}
