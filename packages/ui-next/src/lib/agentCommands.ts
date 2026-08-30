/**
 * The `/` command palette the console dock turns into when the reader's query
 * starts with a slash — pure over injected deps, like `agentSuggestions.ts`
 * beside it: no React, no store, no I/O, so the palette's *contents* can be
 * reasoned about without mounting the dock at all.
 *
 * Design: `docs/superpowers/specs/mock-full-design.md` §F (the agent dock).
 *
 * **`Go` is resource-scoped, not the route table.** §F's own mock reads
 * `Follow logs · checkout-api`, `Open shell in checkout-api`,
 * `Port forward checkout-api` — navigations about the resource the active
 * route names, the same subject `Action` uses, not a way to jump to an
 * app-scoped screen by name. `Action` and `Go` are therefore both absent on a
 * route with no resource (`parseDetailRoute` returns `null`): a command that
 * cannot say what it would act on is not a command, and a missing group is
 * absent, never an empty heading — which falls out for free here, since a
 * group with no `Command` in the flat list below renders no heading at all.
 *
 * **`shell` and `forward` do not carry the mock's copy verbatim — a
 * deliberate deviation, decided in Task 6.** §F's `Open shell in
 * checkout-api` and `Port forward checkout-api` name the resource because the
 * mock's dock never actually navigates anywhere; the real one does, through
 * `openTab`, and neither destination has a way to arrive pre-selected.
 * `NewSessionMenuProps` (`screens/terminals/NewSessionMenu.tsx`) takes only
 * `context` and `namespace` — no pod field at all — and nothing wires a
 * target into `/forwards`' own header action either. Both dialogs open only
 * from a local button click on their own screen; there is no route-level or
 * cross-tab channel today that could hand a resource's identity to a dialog
 * mounted by a *different* screen's navigation, and building one is real
 * plumbing across two screens this step does not own. Landing a reader on
 * `/terminals` under a label that says `Open shell in checkout-api` would be
 * a promise the navigation cannot keep — worse than a generic label that
 * says what actually happens. So the labels below name the SCREEN the
 * command opens, not the resource, until one of those dialogs grows a way to
 * take it. `Follow logs` keeps the resource's name because `logsRoute` bakes
 * the identity into the URL itself — that promise the navigation does keep.
 *
 * **No Roll back.** Core has no rollout-undo capability for a Deployment's
 * revision history — `rolloutRestart`, `scale`, `evict`, `deletePod`,
 * `cordonNode`, `drainNode` and Helm's own `helmRollback` exist, but nothing
 * walks a Deployment's revision history back. A command that refuses the
 * moment it is run is worse than a command that was never drawn, so this
 * module draws none.
 *
 * **A destructive command navigates; it does not open a second dialog over
 * the dock.** `openAction` is the hand-off: Task 6 wires it to the exact
 * confirm the resource's own row menu opens, with the intent (`kind`,
 * `namespace`, `name`, `context`, `action`) carried across. That is what
 * keeps there being exactly one confirm for a mutation, in the tab that owns
 * the resource, rather than a second ungated door floating over the console.
 * `deps.context` is read once, here, into each command's closure at BUILD
 * time — the same pin `useRowMenu`'s `Pending.context` takes and for the same
 * reason (`ResourceMenu.tsx`): the cluster rail can move between the
 * keystroke that built this list and the pick that runs one of its commands,
 * and the write must still reach the cluster the reader read this list on.
 */
import { K8S_KIND } from "@srelens/core";
import { parseDetailRoute } from "./detailRoute";
import { descriptorFor } from "./kinds/descriptors";

export type CommandGroup = "Action" | "Go" | "Cluster" | "Workspace";

export interface Command {
  id: string;
  group: CommandGroup;
  label: string;
  hint: string;
  danger?: true;
  run: () => void;
}

export interface CommandDeps {
  /** The active tab's route — what `Action` and `Go` are scoped to. */
  route: string;
  /** The kubeconfig context in focus, pinned into every command built here. */
  context: string;
  clusters: readonly { id: string; name: string }[];
  workspaces: readonly { id: string; name: string }[];
  /**
   * Opens a route in a tab. Takes the same `clusterName` the real store
   * function does — `ResourceMenu.tsx`'s own Follow-logs / detail / edit
   * navigations always pass `{ clusterName: context }`, so a tab this module
   * opens must too, or it carries no cluster label (or a stale one after a
   * cluster switch) where every other navigation of the same actions does.
   */
  openTab: (route: string, opts?: { clusterName?: string }) => void;
  /**
   * Switches the active cluster. **Both arguments, always**: since the tab
   * strip's relabel fix, a caller that passes only `id` leaves the strip
   * showing the previous cluster's name over the new cluster's tabs.
   */
  setActiveCluster: (id: string, name: string) => void;
  switchWorkspace: (id: string) => void;
  onToggleTheme: () => void;
  /**
   * Opens the row menu's own confirm for a destructive action, with the
   * intent — never the capability — carried across. See the module doc for
   * why this replaces calling `rolloutRestart`/`scaleResource` here directly.
   */
  openAction: (a: {
    kind: string;
    namespace: string;
    name: string;
    context: string;
    action: "scale" | "restart";
  }) => void;
  /**
   * Opens `Follow logs` / `Open shell` / `Port forward` on a resource, with
   * the identity carried across — the same reason `openAction` exists rather
   * than a bare navigation: `Open shell` and `Port forward` have no route of
   * their own (a session and a dialog, not a route), so a bare `openTab`
   * lands on `/terminals` or `/forwards` with nothing to say which resource
   * it was for. Task 6 implements this against the real capability and
   * screen; this module only names the intent.
   */
  openResource: (r: {
    kind: string;
    namespace: string;
    name: string;
    context: string;
    as: "logs" | "shell" | "forward";
  }) => void;
}

/**
 * `k8sKind → slug`, the reverse of core's own `K8S_KIND`, so this module can
 * ask the list screen's own `KindDescriptor` which actions a kind offers —
 * the same question `detailData.tsx`'s `SLUG_BY_K8S_KIND` asks, built the
 * same way (from core's table, not hand-duplicated) for the same reason: a
 * kind added there must never go silently unresolvable here.
 */
const SLUG_BY_K8S_KIND: Record<string, string> = Object.fromEntries(
  Object.entries(K8S_KIND)
    .filter(([, k8sKind]) => k8sKind !== "")
    .map(([slug, k8sKind]) => [k8sKind, slug]),
);

/**
 * `Action` and `Go` — both scoped to the one resource `deps.route` names, and
 * both absent when it names none.
 */
function resourceCommands(deps: CommandDeps): Command[] {
  const detail = parseDetailRoute(deps.route);
  if (!detail) return [];
  const { kind, namespace, name } = detail;
  const ns = namespace ?? "";
  const slug = SLUG_BY_K8S_KIND[kind];
  // A kind absent from `K8S_KIND` (a CRD) resolves no descriptor and offers
  // no `Action`/`Go` command here — the same refusal `KindActions` documents
  // for Delete on a custom resource: offering an action that cannot say what
  // it would run against is worse than not offering it.
  const actions = slug ? (descriptorFor(slug)?.actions ?? {}) : {};
  // Read once, into every command's closure below — see the module doc.
  const context = deps.context;

  const commands: Command[] = [];

  if (actions.restart) {
    commands.push({
      id: "restart",
      group: "Action",
      label: `Restart ${kind}/${name}`,
      hint: "rollout restart",
      danger: true,
      run: () => deps.openAction({ kind, namespace: ns, name, context, action: "restart" }),
    });
  }
  if (actions.scale) {
    commands.push({
      id: "scale",
      group: "Action",
      label: `Scale ${kind}/${name}`,
      hint: "adjust replica count",
      danger: true,
      run: () => deps.openAction({ kind, namespace: ns, name, context, action: "scale" }),
    });
  }

  // Every `Go` command hands the full resource identity to `openResource`
  // rather than opening a bare route: `Open shell` and `Port forward` have no
  // route of their own to carry the subject through (a session and a dialog,
  // not a route), and building that capability here — reaching into
  // `sessions.ts` or the forward dialog directly — is not this step's to do.
  // `Follow logs` goes through the same door for the same reason `openAction`
  // is one door rather than two: one hand-off, fully specified, that Task 6
  // maps to `openTab(logsRoute(...), { clusterName: context })` on its side.
  // `kind`/`namespace`/`name` still travel to `openResource` for `shell` and
  // `forward` too, even though Task 6's own wiring cannot use them today (see
  // the module doc) — a future widening of either destination reads them off
  // this same call rather than a second one threaded through later.
  if (actions.logs) {
    commands.push({
      id: "logs",
      group: "Go",
      label: `Follow logs · ${name}`,
      hint: "all containers",
      run: () => deps.openResource({ kind, namespace: ns, name, context, as: "logs" }),
    });
  }
  if (actions.shell) {
    commands.push({
      id: "shell",
      group: "Go",
      // Not `Open shell in ${name}` — see the module doc's "deliberate
      // deviation" note. This opens Terminals; it does not attach to `name`.
      label: "Open a shell",
      hint: "Terminals · pick the pod there",
      run: () => deps.openResource({ kind, namespace: ns, name, context, as: "shell" }),
    });
  }
  if (actions.forward) {
    commands.push({
      id: "forward",
      group: "Go",
      // Not `Port forward ${name}` — same reason as `shell`, above.
      label: "Port forward",
      hint: "Forwards · pick the target there",
      run: () => deps.openResource({ kind, namespace: ns, name, context, as: "forward" }),
    });
  }

  return commands;
}

/** One `Switch to <cluster>` per cluster the strip knows about — never
 *  filtered to "not the current one": the palette lists what it can do, not
 *  what it thinks the reader wants. */
function clusterCommands(deps: CommandDeps): Command[] {
  return deps.clusters.map((c) => ({
    id: `cluster-${c.id}`,
    group: "Cluster" as const,
    label: `Switch to ${c.name}`,
    hint: c.id,
    // §F: "a Cluster command → /" — the reader lands on the control room
    // rather than a tab that may name nothing on the cluster just switched
    // to. `clusterName` is passed here too, not left to `setActiveCluster`'s
    // own relabel alone: that relabels tabs the workspace ALREADY has, and a
    // workspace with no "/" tab open yet would otherwise mint one with no
    // cluster label at all.
    run: () => {
      deps.setActiveCluster(c.id, c.name);
      deps.openTab("/", { clusterName: c.name });
    },
  }));
}

/**
 * `Switch to <workspace>` per workspace, plus `Toggle theme`.
 *
 * Theme has no group of its own among the design's four (`Action`, `Go`,
 * `Cluster`, `Workspace`) — it is a window-level preference, not scoped to a
 * cluster or a resource, so it sits here rather than inventing a fifth
 * `CommandGroup` member for one command.
 */
function workspaceCommands(deps: CommandDeps): Command[] {
  const switches: Command[] = deps.workspaces.map((w) => ({
    id: `workspace-${w.id}`,
    group: "Workspace",
    label: `Switch to ${w.name}`,
    hint: "workspace",
    run: () => deps.switchWorkspace(w.id),
  }));
  return [
    ...switches,
    {
      id: "theme",
      group: "Workspace",
      label: "Toggle theme",
      hint: "light / dark",
      run: () => deps.onToggleTheme(),
    },
  ];
}

export function commandsFor(deps: CommandDeps): readonly Command[] {
  return [...resourceCommands(deps), ...clusterCommands(deps), ...workspaceCommands(deps)];
}

/** Case-insensitive substring match on the label — nothing else, so a match
 *  the reader can see none of the reason for (a hidden id, a hint) can never
 *  come back. */
export function matchCommands(all: readonly Command[], query: string): readonly Command[] {
  const q = query.trim().toLowerCase();
  if (!q) return all;
  return all.filter((c) => c.label.toLowerCase().includes(q));
}
