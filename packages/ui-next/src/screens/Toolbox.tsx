import { useMemo, useState } from "react";
import { isTauri, plural, startToolInstall, toolboxStatus } from "@srelens/core";
// Named the same as chat's `ToolStatus` (an assistant tool call's state), so
// taken from its own module rather than from the barrel, where the two collide.
import type { ToolStatus } from "@srelens/core/lib/toolbox";
import {
  Alert,
  Button,
  LoadingState,
  Screen,
  SideRail,
  StatusPill,
  Table,
  type Column,
} from "@srelens/ui-kit";
import { useActiveContext, useContexts } from "../lib/clusters";
import { FailureAlert, FailureState } from "../lib/errorCopy";
import { formatBytes } from "../lib/numbers";
import { useInfo } from "../lib/probe";
import { useResource } from "../lib/useResource";
import {
  ExecAuthRail,
  EXEC_AUTH_RAIL_WIDTH,
  TOOL_VERDICT,
  type ToolState,
} from "./toolbox/ExecAuthRail";

/** §17's pane head, verbatim — `SearchPaths` really does add that directory. */
const PANE_HEAD = "Managed tools · installed under ~/.srelens/bin";

/** What a cell shows in place of a fact nobody has. */
const ABSENT = "—";

/**
 * The tools `toolbox.status` reports that srelens can also *install*.
 *
 * `MANAGED_TOOLS` in `crates/kube/src/toolbox.rs` and `startToolInstall`'s own
 * union are the same three today, and this is deliberately the narrower of the
 * two: a fourth entry added to the inventory with no installer behind it gets
 * an inventory row and no button, rather than a button whose click rejects on
 * an unknown tool name. A control that cannot work is not drawn.
 */
const INSTALLABLE = new Set(["kubectl", "helm", "krew"]);
type InstallableTool = "kubectl" | "helm" | "krew";

/**
 * What the screen can tell apart about a tool, and the word it draws for each
 * — {@link TOOL_VERDICT}, which lives beside the rail because the rail needs
 * the same four words and one table is the rule.
 *
 * §17 draws a fifth state, `update` — a newer version exists upstream — and
 * there is no source for it anywhere in this app. Nothing queries a release
 * feed, and a kubeconfig never states a minimum version for the binary it
 * names, so an `update` state could only ever be inferred. It is not inferred
 * here, and its `Update` button went with it.
 *
 * `unmanaged` is the one §17 does not draw and this does: a tool that IS
 * installed and IS usable, but that somebody else put on the PATH. It matters
 * because the pane head promises `~/.srelens/bin`, and a row reading plain
 * `Installed` for a Homebrew helm quietly claims srelens put it there.
 */
function toolState(tool: ToolStatus): ToolState {
  if (!tool.installed) return "missing";
  return tool.source === "system" ? "unmanaged" : "installed";
}

/** A `vMAJOR.MINOR…` string as the pair Kubernetes measures skew in. */
function minorVersion(text: string): { major: number; minor: number } | null {
  const m = /^v?(\d+)\.(\d+)(?:[.+-]|$)/.exec(text.trim());
  return m ? { major: Number(m[1]), minor: Number(m[2]) } : null;
}

/**
 * kubectl's distance from the API server it will be pointed at, in the units
 * Kubernetes states its own skew policy in: minor versions.
 *
 * Empty when either side is unreadable or unread — an app-scoped screen can be
 * opened before any cluster has answered, and a note that said "matches" with
 * no server version in hand would be an assertion about a cluster nobody
 * asked. The comparison is deliberately *only* between kubectl and the server:
 * helm's `v3.16` has nothing to do with the API server's `v1.31`, and counting
 * the difference would produce thirteen minors of nonsense.
 *
 * It names the skew and stops there. Whether a given distance is *supported*
 * is upstream policy that changes release to release, and this screen has no
 * copy of it — the number and its direction are the facts.
 */
function skewNote(clientVersion: string, serverVersion: string, context: string): string {
  const client = minorVersion(clientVersion);
  const server = minorVersion(serverVersion);
  if (!client || !server) return "";
  if (client.major !== server.major) return `differs from ${context} server version`;
  const skew = client.minor - server.minor;
  if (skew === 0) return `matches ${context} server version`;
  return skew > 0
    ? `${plural(skew, "minor")} ahead of ${context}`
    : `${plural(-skew, "minor")} behind ${context}`;
}

/**
 * The Note column — **derived facts only.**
 *
 * Two of them, and they are the two this machine can answer: where the binary
 * came from, and how far kubectl is from the cluster in focus. §17's other
 * notes are not shipped, on purpose:
 *
 * - `optional · multi-pod log tailing` and `cluster add-on · absent on
 *   edge-apac` are opinions about what a tool is FOR, not facts about this
 *   machine, and they would be the same words on every install of srelens.
 * - `12 plugins` costs a whole krew index search: `toolbox.searchPlugins`
 *   returns `{ name, description, installed }` per result and there is no
 *   count behind it, so the number would be paid for by walking the index
 *   every time this screen opened. A figure that expensive does not belong in
 *   a table cell.
 */
function noteFor(tool: ToolStatus, serverVersion: string, context: string): string {
  const notes: string[] = [];
  // Said as the note rather than as the state word: `Unmanaged` says srelens
  // did not put it there, and this says where it is instead.
  if (tool.installed && tool.source === "system") notes.push("on PATH");
  if (tool.name === "kubectl" && tool.version && serverVersion && context) {
    const skew = skewNote(tool.version, serverVersion, context);
    if (skew) notes.push(skew);
  }
  return notes.join(" · ");
}

interface ToolRow {
  name: string;
  version: string;
  state: ToolState;
  note: string;
  size: string;
  installed: boolean;
}

/**
 * `/toolbox` — the design's Toolbox screen (§17).
 *
 * The one screen in the app that never touches a cluster: it inventories the
 * command-line tools srelens drives, says which of them srelens itself
 * installed, and on the desktop offers to (re)install them. Its only reference
 * to Kubernetes is the one derived fact kubectl has that the others do not —
 * how far its minor version is from the server of the cluster in focus, read
 * from the probe the shell already ran at launch rather than from a request of
 * its own.
 *
 * **Three things §17 asks for are deliberately absent.** Each is argued where
 * it would have gone: the `update` state ({@link ToolState}), most of the Note
 * column ({@link noteFor}), and the Size column ({@link COLUMNS}). The fourth
 * departure is the action column, which exists on the desktop and not in the
 * browser — see {@link Toolbox} below.
 *
 * **Loading is a real state here.** `toolbox.status` shells out to locate each
 * binary and run it for its version; on a cold PATH that is not instant, and a
 * table that appeared empty first and filled in later would read as "nothing
 * installed".
 */
export function Toolbox(_props: { route: string }) {
  const context = useActiveContext();
  // The server version, from the probe the shell already ran at launch — the
  // same source the cluster overview's head reads. Nothing here connects.
  const serverVersion = useInfo(context?.stableId ?? null)?.version ?? "";

  // EVERY context, not the active one. A tool this cluster is fine without is
  // exactly what stops the next cluster answering, and the reader with a
  // broken context cannot select it to find that out — selecting it is what
  // fails. The rail memoizes on the names so the store's identity churn does
  // not re-diagnose the kubeconfig on every notification.
  const all = useContexts();
  const contextNames = useMemo(() => all.map((c) => c.name), [all]);

  const inventory = useResource(async () => {
    const result = await toolboxStatus();
    // `toolboxStatus` reports a rejection as `{ error }` rather than throwing,
    // so it is turned back into one: `useResource` owns the four states, and
    // an error smuggled through the data channel would render as a ready table
    // with no rows in it.
    if (result.error) throw new Error(result.error);
    return result.data ?? [];
  }, []);

  /** Which tool is being installed, and how far the download has got. */
  const [busy, setBusy] = useState<string | null>(null);
  const [percent, setPercent] = useState<number | null>(null);
  const [installError, setInstallError] = useState<{ tool: string; error: unknown } | null>(null);

  const desktop = isTauri();

  const rows = useMemo<ToolRow[]>(
    () =>
      (inventory.data ?? []).map((tool) => ({
        name: tool.name,
        version: tool.version || ABSENT,
        size: formatBytes(tool.sizeBytes) || ABSENT,
        state: toolState(tool),
        note: noteFor(tool, serverVersion, context?.name ?? ""),
        installed: tool.installed,
      })),
    [inventory.data, serverVersion, context?.name],
  );

  async function install(name: string) {
    setBusy(name);
    setPercent(null);
    setInstallError(null);
    const result = await startToolInstall(name as InstallableTool, setPercent);
    setBusy(null);
    setPercent(null);
    if (result.error) setInstallError({ tool: name, error: result.error });
    // Re-read either way: a failed install can still have moved the binary,
    // and the row's own words are the only honest report of where it ended up.
    inventory.reload();
  }

  const columns: Column<ToolRow>[] = desktop
    ? [
        ...COLUMNS,
        {
          // §17's unnamed trailing column. One button, whose word is what it
          // will actually do: `Install` for a tool that is not there, and
          // `Reinstall` for one that is — including an unmanaged copy, where
          // installing puts srelens's own under `~/.srelens/bin`. §17's third
          // label, `Update`, went with the state it depended on.
          key: "action",
          header: "",
          sortable: false,
          filterable: false,
          align: "end",
          minWidth: 96,
          render: (row) => {
            if (!INSTALLABLE.has(row.name)) return null;
            const verb = row.installed ? "Reinstall" : "Install";
            const running = busy === row.name;
            return (
              <Button
                variant="secondary"
                size="sm"
                // The word on the button changes while the download runs, so
                // the accessible name is spelled out and stays put — three
                // rows all offering "Install" name nothing at all.
                aria-label={`${verb} ${row.name}`}
                disabled={busy !== null}
                onClick={() => void install(row.name)}
              >
                {running ? progressLabel(percent) : verb}
              </Button>
            );
          },
        },
      ]
    : COLUMNS;

  return (
    <Screen title="Toolbox" eyebrow="workspace / binaries" fill>
      {/* The inventory answers "what does this machine have"; the rail answers
          "and can each context still authenticate", which is the question a
          cluster that will not open is actually asking. `mainHead` keeps §17's
          pane head level with the rail's own. */}
      <SideRail
        head="Exec auth check"
        mainHead={PANE_HEAD}
        width={EXEC_AUTH_RAIL_WIDTH}
        rail={<ExecAuthRail contexts={contextNames} />}
      >
        <div className="scroll flex min-h-0 flex-1 flex-col gap-3 p-3">
          {!desktop && (
            /* Said ONCE, for the whole table, rather than as a disabled button on
               every row. `installKubectl`, `installHelm`, `installKrew`,
               `installPlugin`, `upgradePlugin` and `removePlugin` are all in the
               server's `WEB_DENIED_CAPABILITIES`, so every one of §17's per-row
               buttons would fail here — and a row of controls that reject on
               click is worse than a table that reads as the inventory it is. */
            <Alert tone="info" title="Tools are managed where srelens runs">
              This lists what that machine has. Installing and updating them happens in the srelens
              desktop app.
            </Alert>
          )}
          {installError && (
            <FailureAlert
              tone="sev"
              title={`Could not install ${installError.tool}`}
              error={installError.error}
            />
          )}
          {inventory.status === "loading" ? (
            <LoadingState label="Locating the toolchain" />
          ) : inventory.status === "error" ? (
            <FailureState
              title="Could not inventory the toolchain"
              error={inventory.error}
              onRetry={inventory.reload}
            />
          ) : (
            <Table
              columns={columns}
              data={rows}
              getRowKey={(row) => row.name}
              emptyText="No tools reported"
              emptyHint="srelens found no entry for kubectl, helm or krew — not even a missing one."
            />
          )}
        </div>
      </SideRail>
    </Screen>
  );
}

/** What the button says while an install is running. */
function progressLabel(percent: number | null): string {
  // Null covers both halves of the operation the percentage cannot describe:
  // a download whose total size the server never sent, and the verify-and-move
  // that happens after the last byte arrives.
  return percent === null ? "Installing…" : `Downloading… ${percent}%`;
}

/**
 * The columns every platform draws, in §17's order — **minus `Size`.**
 *
 * §17 puts a right-aligned size on every row (`54.2 MB`, `48.9 MB`, `9.1 MB`).
 * `Size` reads the byte length of the file at `path`, following symlinks —
 * these entries are usually symlinks, and a link's own length is a handful of
 * bytes: a plausible-looking wrong number, which is worse than none.
 *
 * A tool with no readable size renders a dash, never `0 B`. A zero is a
 * measurement, and a tool that is not installed — or whose path cannot be
 * stat'd — has not been measured. That rule has held from the cluster overview
 * through every screen since.
 */
const COLUMNS: Column<ToolRow>[] = [
  {
    key: "name",
    header: "Tool",
    render: (row) => <span className="font-medium">{row.name}</span>,
  },
  {
    key: "version",
    header: "Version",
    render: (row) => <span className="tabular-nums">{row.version}</span>,
  },
  {
    key: "state",
    header: "State",
    // Sorted and searched on the word the reader can see, not on the internal
    // id behind it — a filter for "missing" that matched nothing because the
    // cell says `Missing` is the sort of mismatch nobody reports.
    getValue: (row) => TOOL_VERDICT[row.state].word,
    render: (row) => (
      <StatusPill status={TOOL_VERDICT[row.state].word} kind={TOOL_VERDICT[row.state].kind} />
    ),
  },
  {
    key: "note",
    header: "Note",
    render: (row) => <span className="text-muted">{row.note}</span>,
  },
  {
    key: "size",
    header: "Size",
    align: "end",
    render: (row) => <span className="tabular-nums text-muted">{row.size}</span>,
  },
];
