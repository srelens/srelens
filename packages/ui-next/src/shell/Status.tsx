import { useSyncExternalStore } from "react";
import {
  getForwards,
  isForwardEnded,
  plural,
  subscribeForwards,
  type ClusterContext,
} from "@srelens/core";
import { StatusBar, type StatusSegment } from "@srelens/ui-kit";
import { useConsole } from "../console";
import { getHelmOps, subscribeHelmOps } from "../lib/helmOps";
import { useInfo } from "../lib/probe";
import { getSessions, subscribeSessions } from "../lib/sessions";
import { openTab, useActiveCluster } from "../lib/tabsStore";
// The words and their tones live beside `LinkState` rather than here: the
// overview rail reads the same link and must say the same thing about it.
import { LINK_TONE, LINK_WORD, useWorkspaceView } from "../lib/workspace";

/**
 * The strip along the bottom of the window: which cluster this window is
 * looking at, what version it runs, whether it is reachable, how many
 * port-forwards are up, how many shells are still running, and the way in to
 * the console.
 *
 * Every readout here is somebody else's fact — the tab store's active cluster,
 * the probe's version, the workspace view's link state, core's forwards — so
 * this component is only the place they meet. The kit draws them, and it is
 * not allowed to know what a cluster or a forward is, which is why they arrive
 * as segments rather than as props with those names on them.
 *
 * `contexts` is passed in rather than read from a store because the store holds
 * `stableId`s and the strip shows names: the id survives a rename and the name
 * is what a person recognises, and only the caller that listed the contexts can
 * turn one into the other.
 *
 * The forwards count comes straight off core's module-level store rather than
 * through a hook of ours. It is shared with the per-resource "Forward" action
 * and must survive this component unmounting, so subscribing to it is the whole
 * of the wiring — a copy in ui-next would be a second answer to the same
 * question.
 *
 * The shell count follows the same shape, off `lib/sessions`' store instead —
 * that one is ui-next's own rather than core's (it holds the xterm instance),
 * but read the same way: `useSyncExternalStore` over a module-level snapshot,
 * so a session outliving the Terminals screen keeps being counted here too.
 *
 * The helm readouts are the third of these, and the one with the least excuse
 * for being anywhere else: a `helm upgrade --wait` runs for minutes and
 * outlives the dialog that started it, so once that dialog closes this strip
 * is the only surface reporting a cluster that is still being changed.
 *
 * All three snapshots are read raw and counted afterwards. Filtering inside a
 * `useSyncExternalStore` getter would hand React a fresh array on every read,
 * which it compares by identity — "Maximum update depth exceeded", and a
 * streaming helm operation prints often enough to make that a hang rather
 * than a waste. The stores promise a stable reference; the arithmetic below
 * is what keeps that promise useful.
 */
export function Status({ contexts }: { contexts: ClusterContext[] }) {
  const activeId = useActiveCluster();
  const info = useInfo(activeId);
  const { links } = useWorkspaceView();
  const { setOpen } = useConsole();
  const forwards = useSyncExternalStore(subscribeForwards, getForwards, getForwards);
  const sessions = useSyncExternalStore(subscribeSessions, getSessions, getSessions);
  const helmOps = useSyncExternalStore(subscribeHelmOps, getHelmOps, getHelmOps);

  // Found rather than assumed: the active id is persisted and the context list
  // is whatever the machine has now, so an id can outlive the context it named.
  const ctx = contexts.find((c) => c.stableId === activeId);
  // Nothing has probed yet, or there is nothing to probe. Either way the link
  // is not up, and "Disconnected" is the honest reading of that.
  const state = (activeId ? links[activeId]?.state : undefined) ?? "disconnected";
  // Split rather than counted blind. A tunnel that gave up is still in the
  // store — it stays on the forwards screen until its reader dismisses it —
  // and counting it here would report a dead tunnel as one of the ones
  // carrying traffic, which is the assumption that made a dead forward
  // dangerous in the first place.
  const dead = forwards.filter(isForwardEnded).length;
  const n = forwards.length - dead;
  // Idle is running — a session naps after a minute of quiet and is still
  // there to type into. Only `closed` is dead, and a closed row stays on the
  // rail to show what died and why; counting it here would say a shell is
  // alive when it is not, the same lesson the tunnel above already carries.
  const liveSessions = sessions.filter((s) => s.state !== "closed").length;
  // In flight means `running` and nothing else. A `done` operation has stopped
  // changing the cluster and a `failed` one has stopped trying; both stay
  // listed on the helm screen with their output, and counting either here
  // would report a finished mutation as one still under way — the distinction
  // the dead tunnel and the closed shell above are each already drawing.
  const runningOps = helmOps.filter((o) => o.state === "running").length;
  const failedOps = helmOps.filter((o) => o.state === "failed").length;

  const segments: StatusSegment[] = [
    {
      id: "ctx",
      label: ctx?.name ?? "No cluster",
      dot: true,
      tone: LINK_TONE[state],
      // Pressable only when there is a cluster to open. A "No cluster" button
      // that opens an overview of nothing is a dead end dressed as a way out.
      onSelect: ctx ? () => openTab("/overview", { clusterName: ctx.name }) : undefined,
    },
  ];
  if (ctx) {
    // The probe may not have landed, and a reachable cluster can still report
    // no version. Both are "we do not know yet", said in words rather than by
    // dropping the readout — a segment that comes and goes moves the ones after
    // it along the strip every time a cluster is probed.
    segments.push({ id: "ver", label: info?.version ?? "version unknown" });
  }
  // Pulsing only while connecting: the dot is animated for a readout that is
  // still changing, not for one that merely happens to be current.
  segments.push({ id: "link", label: LINK_WORD[state], pulse: state === "connecting" });

  const end: StatusSegment[] = [];
  // Only when there is one, and this is the exception to the rule the version
  // segment states above: a readout that comes and goes moves what follows it
  // along the strip, which is a cost worth paying exactly once — for the news
  // that a tunnel the reader is depending on has died. The count beside it
  // cannot carry that: a forward that dies takes the strip to `0
  // port-forwards`, which is what a reader with no forwards at all sees.
  if (dead > 0) {
    end.push({
      id: "pf-dead",
      label: `${plural(dead, "forward")} failed`,
      tone: "sev",
      dot: true,
      onSelect: () => openTab("/forwards"),
    });
  }
  end.push({
    id: "pf",
    label: `${n} port-forward${n === 1 ? "" : "s"}`,
    tone: "info",
    // A dot for "something is running", so the strip reads as live at a
    // glance; none at zero, where there is nothing to be live about.
    dot: n > 0,
    onSelect: () => openTab("/forwards"),
  });
  // Absent at zero, same as the dead-forward segment above: a `0 shells`
  // readout is noise on a strip already carrying five of them, and there is
  // nothing live to send the reader to `/terminals` for.
  if (liveSessions > 0) {
    end.push({
      id: "shells",
      label: plural(liveSessions, "shell"),
      tone: "info",
      dot: true,
      onSelect: () => openTab("/terminals"),
    });
  }
  // The `pf-dead` exception again, and for a bigger stake than a tunnel. A
  // `helm upgrade` that failed is a cluster mutation that half-happened, and
  // the dialog that would have said so has closed — this is the only surface
  // left that reports it at all. The count beside it cannot: a failed
  // operation leaves nothing in flight, which is exactly what a reader who
  // started no operation sees.
  //
  // Neither of these labels is a status word, so neither wants one from core:
  // `helmStatus` tones the word Helm puts in a release Secret, and these are
  // counts of what this window is running. The tones are the strip's own,
  // shared with the two readouts above — `sev` for news of a failure, `info`
  // for a count of live work — rather than a helm vocabulary invented here.
  if (failedOps > 0) {
    end.push({
      id: "helm-dead",
      label: `${plural(failedOps, "helm operation")} failed`,
      tone: "sev",
      dot: true,
      onSelect: () => openTab("/helm"),
    });
  }
  // Absent at zero, following the shells segment rather than the forwards one:
  // a reader with no helm operation has nothing to be sent to `/helm` for, and
  // `0 helm operations` is the longest way yet of saying nothing.
  if (runningOps > 0) {
    end.push({
      id: "helm",
      label: plural(runningOps, "helm operation"),
      tone: "info",
      dot: true,
      onSelect: () => openTab("/helm"),
    });
  }
  end.push({ id: "ask", label: "Ask", tone: "accent", onSelect: () => setOpen(true) });

  return <StatusBar segments={segments} end={end} />;
}
