import { useEffect, useMemo, useState, useSyncExternalStore } from "react";
import {
  ageFromTimestamp,
  browsable,
  forwardAddress,
  getForwards,
  isForwardEnded,
  kindToForwardTarget,
  openExternal,
  plural,
  rehydrateForwards,
  stopPortForward,
  subscribeForwards,
  toKubectl,
  type ActiveForward,
} from "@srelens/core";
import {
  Badge,
  Button,
  ClipboardCopyStatus,
  EmptyState,
  IconButton,
  Screen,
  StatusPill,
  Table,
  useClipboardCopy,
  type Column,
  type StatusKind,
} from "@srelens/ui-kit";
import { useActiveContext } from "../lib/clusters";
import { FailureAlert, FailureWord } from "../lib/errorCopy";
import { Icons } from "../lib/icons";
import { formatBytes } from "../lib/numbers";
import { useClusterGate } from "../lib/clusterMoved";
import { NewForwardDialog } from "./forwards/NewForwardDialog";

/**
 * THE ONE HAND-PAIRED WORD/TONE TABLE ON THIS SCREEN, AND IT IS MARKED BECAUSE
 * IT SHOULD NOT HAVE TO EXIST.
 *
 * `packages/core` has no verdict for a port-forward's status. It has one for a
 * log stream's connection (`logConnectionStatus`) and two for Kubernetes
 * resources (`k8sStatus`, `k8sHealth`), and a tunnel is none of those three:
 * its states are `active | reconnecting | failed`, which is a different union
 * from `connecting | live | reconnecting | error` and means different things —
 * a log stream that is "live" is being read, a forward that is "active" may
 * have moved nothing all day. Widening either of the existing unions to cover
 * this would put four words on a screen that can only ever show three.
 * **If a `forwardStatus` verdict is ever added to `packages/core`, delete this
 * and call it** — and do not spell `status === "active" ? … : …` anywhere else.
 *
 * §13's rule is "`active`→ok, else warn + bold". `failed` is shipped as
 * `danger` rather than `warning` on purpose: §13 draws only the first two
 * states and its "else" was written about `reconnecting`. A tunnel that is
 * reconnecting is coming back; one that has failed is gone, and dismissing
 * the row is the only thing left to do about it. Amber for both would say the
 * two are the same news.
 *
 * `failed` needed no new word for the dead row: it IS the dead state — core's
 * `isForwardEnded` says so, and this table already had the word for it. A
 * second entry saying "Ended" would have been the same news under two names.
 *
 * `tinted` follows the design's asymmetric colouring rule, which
 * {@link StatusPill} owns: the bad states colour and embolden the word, the
 * good one reads plain. It is decided here, beside the word, so a copy-paste
 * cannot pair "Active" with the emphasis a failure gets.
 */
const FORWARD_VERDICT: Record<ActiveForward["status"], { word: string; kind: StatusKind }> = {
  active: { word: "Active", kind: "success" },
  reconnecting: { word: "Reconnecting", kind: "warning" },
  failed: { word: "Failed", kind: "danger" },
};

/**
 * kubectl's own short target forms, which is what §13 writes in the Target
 * column (`svc/checkout-api`, `pod/search-indexer-0`).
 *
 * Core's `kindToForwardTarget`, which is also what `toKubectl` puts in the
 * command this row copies and what §A.4's dialog names its options with. This
 * screen used to keep a private `{ Service: "svc", Pod: "pod" }` table beside
 * it; three consumers of one rule beats three copies of it, and a drift would
 * have had the cell and the clipboard disagreeing about what a row is.
 */
function targetOf(kind: string, name: string): string {
  return `${kindToForwardTarget(kind)}/${name}`;
}

/**
 * How often the Age column recomputes.
 *
 * A screen of live tunnels is the one place in the app where a frozen age is a
 * lie the reader would act on — "18m" under a forward that died an hour ago.
 * Every other Age in the app is re-read when its list is re-fetched; nothing
 * re-fetches here, because the store is pushed to. A second is the resolution
 * of core's own age words below a minute, and the backend already pushes a
 * traffic total about that often, so this costs nothing a live forward was not
 * already costing.
 */
const AGE_TICK_MS = 1_000;

/** One row of §13's table: every cell already resolved to what it renders. */
interface ForwardRow {
  id: number;
  /** `svc/checkout-api` — kubectl's name for the thing, not Kubernetes's. */
  target: string;
  namespace: string;
  cluster: string;
  /**
   * Where this forward is reachable FROM THE MACHINE READING THIS PAGE —
   * `forwardAddress`, never a locally assembled `localhost:<port>`. See
   * {@link Forwards}.
   */
  address: string;
  remote: string;
  status: ActiveForward["status"];
  /**
   * This tunnel has given up: it is on the screen to be READ, not used. Core
   * decides it — the status bar counts the same rows and must agree — and the
   * row asks it here rather than spelling out a status comparison, which is
   * the rule {@link FORWARD_VERDICT} states.
   */
  dead: boolean;
  /** Why it gave up, raw, when the backend said. `describeError` turns it
   *  into a sentence at the cell; see {@link forwardColumns}. */
  error: string | undefined;
  traffic: string;
  bytesMoved: number;
  age: string;
  startedAt: number;
  command: string;
}

/**
 * `/forwards` — the design's Port forwards screen (§13).
 *
 * A port-forward pipes a local port to a Pod or a Service; this is the list of
 * them, and the only place in the app that can stop one. There is no fetch
 * here: `packages/core`'s forwards store is module-level and pushed to by the
 * backend, so the table is a `useSyncExternalStore` over it.
 *
 * **A tunnel that dies stays listed.** One the reader stopped disappears at
 * once — they did it, and it does not need reporting back — but one that gave
 * up underneath them is news, and deleting the row left a forward that had
 * been up for fifteen minutes gone with no trace and its reader still
 * depending on it. Such a row reads `Failed`, says why, counts against
 * neither of the two live counts, and offers a dismissal in place of a stop.
 *
 * **Two things §13 asks for are deliberately not shipped as written.**
 *
 * §13's Copy URL writes `http://localhost:<local>`. That is the DESKTOP answer
 * and only the desktop answer: in web mode srelens runs in a container whose
 * loopback the browser cannot reach, and the address is a same-origin
 * `/pf/<id>/` proxy instead. `forwardAddress` in core already decides this, and
 * the button and the Local cell both read it rather than re-deriving it — so
 * what the row shows and what the clipboard gets cannot disagree. The Local
 * column shows that same address for the same reason: a column headed `Local`
 * printing a port nothing on this machine is listening on is worse than a long
 * URL.
 *
 * §13 also says, in as many words, that the design defines an empty state for
 * this screen in the Components gallery and never renders it here. It renders
 * here. A reader with no tunnels needs the sentence about what a forward is
 * and the button that makes one far more than a reader with four does.
 *
 * **The age ticks.** See {@link AGE_TICK_MS}.
 */
export function Forwards(_props: { route: string }) {
  const forwards = useSyncExternalStore(subscribeForwards, getForwards, getForwards);
  const cluster = useActiveContext();

  /**
   * Adopt whatever the backend is still forwarding.
   *
   * This is what makes the screen honest after a browser reload. The store is
   * module-level JavaScript and a reload empties it; `ForwardManager` on the
   * other side does not die, so without this a web reader reloads into an
   * empty table while their tunnels keep running and nothing in the app can
   * stop them. `rehydrateForwards` never rejects — a failed listing is
   * reported to the reader by core itself — which is why it is called bare.
   */
  useEffect(() => {
    void rehydrateForwards();
  }, []);

  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const tick = setInterval(() => setNow(Date.now()), AGE_TICK_MS);
    return () => clearInterval(tick);
  }, []);

  /**
   * The last thing this screen was refused, in the words the reader gets.
   *
   * One slot rather than two: a stop and an open cannot both be the most
   * recent, and a second banner over a table is a row of the table gone.
   */
  const [failure, setFailure] = useState<{ title: string; error: unknown } | null>(null);
  const clipboard = useClipboardCopy();

  const rows = useMemo<ForwardRow[]>(
    () =>
      forwards.map((f) => ({
        id: f.id,
        target: targetOf(f.kind, f.name),
        namespace: f.namespace,
        cluster: f.context,
        address: forwardAddress({ id: f.id, localPort: f.localPort }),
        // The remote port reads as a port, `:443`, which is how the design
        // writes the far end of a forward and how kubectl's own output does.
        remote: `:${f.remotePort}`,
        status: f.status,
        dead: isForwardEnded(f),
        error: f.error,
        traffic: formatBytes(f.bytesMoved),
        bytesMoved: f.bytesMoved,
        // Core's compact age, from the backend's own start stamp. §13 writes
        // `2h 04m`; every other Age in this app is one value and one unit, and
        // a screen that spelled minutes-past-the-hour here would be the only
        // one — the extra precision is not worth a second way of saying an age.
        age: ageFromTimestamp(new Date(f.startedAt).toISOString(), now),
        startedAt: f.startedAt,
        command: toKubectl({
          action: "port-forward",
          kind: f.kind,
          name: f.name,
          context: f.context,
          namespace: f.namespace,
          localPort: f.localPort,
          remotePort: f.remotePort,
        }),
      })),
    [forwards, now],
  );

  /**
   * The head counts LIVE tunnels, and says how many died apart from them.
   *
   * A dead row is on the screen so it can be read, not because anything is
   * being forwarded through it — so "Active tunnels · 4" over three live ones
   * and a corpse would be exactly the reassurance this screen exists to stop
   * giving. The cluster count follows the same rows for the same reason: the
   * sentence is about what is up, and a cluster whose only tunnel is dead has
   * nothing up in it.
   */
  const live = useMemo(() => rows.filter((r) => !r.dead), [rows]);
  const dead = rows.length - live.length;
  const clusters = useMemo(() => new Set(live.map((r) => r.cluster)).size, [live]);
  // Every row, including the dead ones: those bytes really crossed those
  // tunnels, and a total that shrank when a forward died would read as data
  // lost rather than as a tunnel gone.
  const moved = useMemo(() => rows.reduce((sum, r) => sum + r.bytesMoved, 0), [rows]);

  /**
   * §A.4's dialog, opened from the header action and from the empty state's
   * way out — one handler behind both, so a reader with no tunnels reaches the
   * same dialog as one with four.
   *
   * **The cluster is captured here, when the reader ASKS for the dialog**, not
   * read live while it is open. Since #357 a dialog covers only its own tab, so
   * the rail is live behind this one and `setActiveCluster` switches the active
   * cluster in place with nothing remounting. Read live, everything the dialog
   * had already been told followed the rail: the namespace and target the
   * reader picked out of one cluster's listings stayed in the fields while
   * `Start forward` went to another cluster — a tunnel to staging under a name
   * read off production. `ResourceMenu`'s door into this same dialog pinned it
   * for exactly that reason (see `Forwarding` there); this is the other door.
   */
  const [newForward, setNewForward] = useState<{ context: string; namespace?: string } | null>(null);

  /**
   * What the dialog says, and asks again, when the rail moves out from under
   * it — `lib/clusterMoved`'s gate, wired exactly as the row menu's forward
   * door wires it. A forward exposes a port and then reports it as up, which is
   * why this re-arms the confirmation rather than only stating the divergence.
   */
  const forwardGate = useClusterGate({
    // An EMPTY name on either side is "nothing to compare", not a cluster.
    // `useClusterGate` reads only `null` that way, so a door that pinned
    // `cluster?.name ?? ""` armed a divergence between no cluster and whatever
    // the reader selected next: "This still runs against , not prod-eu", a tick
    // offering to "still forward on .", and a refusal naming neither side. Both
    // ends are legitimate states of this screen — it lists every cluster's
    // forwards, so it is reachable with nothing in focus — and neither is a
    // divergence. A dialog opened with no cluster says so itself, at the top
    // (see `NewForwardDialog`); a rail that later loses its selection changes
    // nothing about the cluster the forward was pinned to.
    pinned: cluster && newForward?.context ? newForward.context : null,
    live: cluster?.name ?? "",
    verb: "forward",
  });

  const openNewForward = () => {
    forwardGate.reset();
    setNewForward({ context: cluster?.name ?? "", namespace: cluster?.namespace });
  };

  const newForwardButton = (
    <Button variant="primary" size="sm" onClick={openNewForward}>
      New forward
    </Button>
  );

  async function stop(row: ForwardRow) {
    setFailure(null);
    try {
      await stopPortForward(row.id);
    } catch (e) {
      setFailure({ title: `Could not stop ${row.target}`, error: e });
    }
  }

  /**
   * Take a dead row off the screen.
   *
   * The SAME command as a stop, and that is not laziness. A forward that gave
   * up on its own stays in the backend's map — and in the listing this screen
   * rehydrates from — until `stop` is called, so a dismissal that only
   * dropped the row here would be undone by the reader's next reload. There
   * is nothing left to abort; `stop` is what makes the backend forget it.
   * Only the words differ, because only the words are different: the reader
   * is not stopping anything, they are acknowledging news.
   */
  async function dismiss(row: ForwardRow) {
    setFailure(null);
    try {
      await stopPortForward(row.id);
    } catch (e) {
      setFailure({ title: `Could not dismiss ${row.target}`, error: e });
    }
  }

  /**
   * Open a tunnel in the reader's own browser.
   *
   * `openExternal`, never an `<a target="_blank">` and never `window.open`:
   * both of those are silent no-ops inside the Tauri WebView (#348), which is
   * exactly the shape of dead link this migration keeps deleting. The address
   * is the row's own — the one `forwardAddress` produced and the cell prints —
   * through `browsable`, because the desktop's answer is a bare authority and
   * a bare authority is not a URL.
   */
  async function openAddress(row: ForwardRow) {
    setFailure(null);
    try {
      await openExternal(browsable(row.address));
    } catch (e) {
      setFailure({ title: `Could not open ${row.target}`, error: e });
    }
  }

  const columns: Column<ForwardRow>[] = [
    // `void`, the way every other handler on this screen discards its
    // promise: the failure is already the banner's business.
    ...forwardColumns((row) => void openAddress(row)),
    {
      // §13's unnamed trailing column. Three compact controls rather than the
      // inline `CopyCommand` §13 names: that component prints the whole command
      // beside its button and refuses to truncate it, which is right in a rail
      // and is a paragraph per row in a 128px column. The command is still one
      // click away, and it is still core's `toKubectl` string verbatim.
      key: "actions",
      header: "",
      sortable: false,
      filterable: false,
      align: "end",
      minWidth: 128,
      render: (row) => {
        const commandKey = `forward-command/${row.id}`;
        const addressKey = `forward-address/${row.id}`;
        const commandStatus = clipboard.statusFor(commandKey);
        const addressStatus = clipboard.statusFor(addressKey);
        return (
          <div className="flex items-center justify-end gap-0.5">
            <IconButton
              icon={
                commandStatus === "copied"
                  ? Icons.check
                  : commandStatus === "failed"
                    ? Icons.warn
                    : Icons.terminal
              }
              // Named per row: four rows all offering "Copy" name nothing at all.
              // The name carries the TARGET, which the row already shows — never
              // the command or the address, which it must not hide in a title.
              label={
                commandStatus === "copied"
                  ? `Copied kubectl command for ${row.target}`
                  : commandStatus === "failed"
                    ? `Copy failed for ${row.target}`
                    : `Copy kubectl command for ${row.target}`
              }
              onClick={() => void clipboard.write(commandKey, row.command)}
            />
            <IconButton
              icon={
                addressStatus === "copied"
                  ? Icons.check
                  : addressStatus === "failed"
                    ? Icons.warn
                    : Icons.copy
              }
              label={
                addressStatus === "copied"
                  ? `Copied address for ${row.target}`
                  : addressStatus === "failed"
                    ? `Copy address failed for ${row.target}`
                    : `Copy address for ${row.target}`
              }
              onClick={() => void clipboard.write(addressKey, row.address)}
            />
            {/* A tunnel that died has nothing left to stop, and a Stop that
                stops nothing is the kind of control this migration keeps
                deleting. Not `danger` either: dismissing news is not a
                destructive act, and the row is already red where it counts. */}
            {row.dead ? (
              <IconButton
                icon={Icons.close}
                label={`Dismiss ${row.target}`}
                onClick={() => void dismiss(row)}
              />
            ) : (
              <IconButton
                icon={Icons.close}
                danger
                label={`Stop forwarding ${row.target}`}
                onClick={() => void stop(row)}
              />
            )}
          </div>
        );
      },
    },
  ];

  return (
    <Screen title="Port forwards" eyebrow="all clusters" actions={newForwardButton} fill>
      {/* Beside the body rather than inside either branch of it, so the dialog
          opens the same from a populated screen and an empty one. */}
      {newForward && (
        <NewForwardDialog
          // The cluster that was in focus when the dialog was asked for, not
          // the one in focus now. A forward is made in one cluster even though
          // this screen lists every cluster's, and the rail's selection at the
          // moment of the gesture is the only answer the app has to *which*;
          // the dialog says so itself when there was none rather than being
          // opened against an empty context.
          context={newForward.context}
          namespace={newForward.namespace}
          moved={forwardGate.alert}
          refusal={forwardGate.refusal}
          onClose={() => {
            setNewForward(null);
            forwardGate.reset();
          }}
        />
      )}
      {failure && (
        <div className="p-3 pb-0">
          <FailureAlert tone="sev" title={failure.title} error={failure.error} />
        </div>
      )}
      <ClipboardCopyStatus feedback={clipboard.feedback} />
      {rows.length === 0 ? (
        <EmptyState
          title="No port forwards"
          hint="Forward a service port to reach it from this machine. Nothing is exposed outside your laptop."
          action={newForwardButton}
          // `fill` hands the body the whole area and leaves the centring to
          // whatever is in it; without this the state sits at the top edge.
          className="flex-1"
        />
      ) : (
        <>
          <div className="pane-head">
            <span>
              {`Active tunnels · ${live.length} across ${plural(clusters, "cluster")}` +
                // Said only when there is one to say: a permanent `· 0 failed`
                // is a readout nobody needs and one more thing to skip past.
                (dead > 0 ? ` · ${dead} failed` : "")}
            </span>
            {/* Pushes the badge to the far end without either side needing to
                know how wide the other is. */}
            <span className="flex-1" />
            <Badge tone="ok">{`${formatBytes(moved)} moved`}</Badge>
          </div>
          <div className="scroll min-h-0 flex-1">
            <Table columns={columns} data={rows} getRowKey={(row) => String(row.id)} />
          </div>
        </>
      )}
    </Screen>
  );
}

/**
 * §13's columns, in §13's order.
 *
 * Module-level so the sort and filter values are read off the same strings the
 * reader sees — a filter for `staging` that matched nothing because the cell
 * was built from a different field is the sort of mismatch nobody reports. The
 * two numeric columns are the exception and say why at each one.
 *
 * It takes ONE handler, for the one cell that does something: the Local
 * address opens. A factory rather than a spliced-in column, so the order stays
 * §13's order in one readable list rather than depending on an index.
 */
function forwardColumns(openAddress: (row: ForwardRow) => void): Column<ForwardRow>[] {
  return [
    {
      key: "target",
      header: "Target",
      // Both lines are searchable: a reader looking for `checkout` means either
      // the service or the namespace and should not have to know which.
      getValue: (row) => `${row.target} ${row.namespace}`,
      render: (row) => (
        <div className="flex min-w-0 flex-col">
          <span className="truncate font-medium">{row.target}</span>
          <span className="path truncate">{row.namespace}</span>
        </div>
      ),
    },
    {
      key: "cluster",
      header: "Cluster",
      // `block`, not a bare span: `truncate` sets `overflow: hidden`, which does
      // nothing to an inline box. A kubeconfig context name is user-chosen and
      // routinely long — `m01-1786968575165/kubernetes-admin@cluster.local` —
      // and without this it draws straight over the Local cell beside it.
      render: (row) => <span className="path block truncate">{row.cluster}</span>,
    },
    {
      key: "address",
      header: "Local",
      /**
       * The one cell that is a control.
       *
       * A `Button`, not an anchor: `<a target="_blank">` opens NOTHING in the
       * Tauri WebView and says nothing about it (#348) — classic's
       * `PortForwardsView` ships that exact dead link today. The kit has no link
       * variant, and `ghost` is the borderless one, so the cell keeps the column's
       * `code` face and reads as text that can be pressed rather than as a box
       * around every row.
       *
       * Its accessible name is the address it shows, which is what a link's would
       * be. No `title`: the address is already on screen, and a value in a title
       * is the rule a Secret once leaked through.
       */
      render: (row) =>
        // A dead tunnel's address answers nothing, so it stops being a
        // control — offering to open it is the #348 shape this screen exists
        // to avoid, and worse than a no-op: it would raise a browser tab onto
        // a refused connection. It stays readable, because the reader has to
        // see WHICH tunnel died.
        row.dead ? (
          <span className="code block truncate text-muted">{row.address}</span>
        ) : (
          <Button
            variant="ghost"
            size="xs"
            className="-mx-1 max-w-full text-[0.8125rem] text-accent"
            onClick={() => openAddress(row)}
          >
            {/* `min-w-0` as well as `truncate`: a flex item's implicit
                `min-width: auto` refuses to shrink below its content, so
                without it a long proxy URL widens the button instead of
                ellipsing — invisible in jsdom, which is why the class is
                asserted. */}
            <span className="code min-w-0 truncate">{row.address}</span>
          </Button>
        ),
    },
    {
      key: "remote",
      header: "Remote",
      // Sorted as the number it is: `:443` beside `:8080` and `:9090` orders
      // 443, 8080, 9090 numerically and "443", "8080", "9090" the same way by
      // luck — `:6060` and `:443` do not.
      getSortValue: (row) => Number(row.remote.slice(1)),
      render: (row) => <span className="tabular-nums">{row.remote}</span>,
    },
    {
      key: "state",
      header: "State",
      // Room for the sentence under a dead tunnel's word. Without it the
      // column sizes to "Reconnecting" and the reason wraps a word per line.
      minWidth: 160,
      // Sorted and searched on the word the reader can see, not on the internal
      // status behind it.
      getValue: (row) => FORWARD_VERDICT[row.status].word,
      /**
       * The word, and under a dead tunnel the reason it gave.
       *
       * `FailureWord` is the kit's answer for a failure in a narrow place: it
       * prints `describeError`'s CLASSIFICATION — "Not authorized", "Can't
       * reach the cluster" — and folds the string the cluster actually sent
       * away behind a disclosure. The raw form is `ApiError: Unauthorized
       * (Status { metadata: Some(ListMeta { … })`, which is unreadable in a
       * table cell and is exactly what a reader needs in a bug report; a
       * `title` attribute would be neither, and is the rule a Secret leaked
       * through.
       *
       * Only when the tunnel is dead AND said why. A reconnecting tunnel is
       * coming back and a reason under it would read as gone, and a closure
       * that carried no reason gets the one word it has rather than an
       * invented sentence.
       */
      render: (row) => (
        <div className="flex min-w-0 flex-col items-start">
          <StatusPill
            status={FORWARD_VERDICT[row.status].word}
            kind={FORWARD_VERDICT[row.status].kind}
            tinted
          />
          {row.dead && row.error !== undefined && (
            <FailureWord error={row.error} className="mt-1 text-[0.75rem] text-muted" />
          )}
        </div>
      ),
    },
    {
      key: "traffic",
      header: "Traffic",
      align: "end",
      // The bytes, not the words: `312 KB` sorts above `44.1 MB` as text.
      getSortValue: (row) => row.bytesMoved,
      render: (row) => <span className="tabular-nums">{row.traffic}</span>,
    },
    {
      key: "age",
      header: "Age",
      align: "end",
      // The start stamp, not the compact age: `2h` sorts below `51m` as text,
      // which is the defect `ageSeconds` exists for — and this row has the
      // original millis, so it does not need to parse its own words back.
      getSortValue: (row) => row.startedAt,
      render: (row) => <span className="tabular-nums text-muted">{row.age}</span>,
    },
  ];
}
