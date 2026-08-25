// @vitest-environment node
import { describe as suite, it, expect } from "vitest";
import { describe, isBuiltInKind, screenFor } from "./routes";
import { AppLog } from "../screens/AppLog";
import { Events } from "../screens/Events";
import { ReleaseNotes } from "../screens/ReleaseNotes";
import { Overview } from "../screens/Overview";
import { Forwards } from "../screens/Forwards";
import { Logs, logsRoute } from "../screens/Logs";
import { ResourceDetailScreen, Resources } from "../screens/Resources";
import { Toolbox } from "../screens/Toolbox";
import { Workloads } from "../screens/Workloads";

suite("isBuiltInKind", () => {
  it("recognises a built-in list kind by its slug", () => {
    expect(isBuiltInKind("pods")).toBe(true);
    expect(isBuiltInKind("deployments")).toBe(true);
  });

  it("does not count overview, which is a kind in core but not a list", () => {
    // `K8S_KIND.overview` is "" — it has no Kubernetes kind behind it, and its
    // route is /overview, not /k/overview.
    expect(isBuiltInKind("overview")).toBe(false);
  });

  it("treats anything else as a custom resource slug", () => {
    expect(isBuiltInKind("certificates.cert-manager.io")).toBe(false);
  });
});

suite("describe", () => {
  it("names the home route and pins it", () => {
    const info = describe("/", "prod-eu");
    expect(info).toMatchObject({ route: "/", title: "Control room", kind: "control", pinned: true });
  });

  it("uses the real cluster name as the sub for cluster-scoped routes", () => {
    // Not the mock's hard-coded "prod-eu".
    expect(describe("/k/pods", "staging-1").sub).toBe("staging-1");
    expect(describe("/logs", "staging-1").sub).toBe("staging-1");
  });

  it("gives app-scoped routes no sub at all", () => {
    for (const route of ["/applog", "/notes", "/settings", "/connections", "/connect", "/toolbox"]) {
      expect(describe(route, "staging-1").sub, route).toBeUndefined();
    }
  });

  it("titles a built-in kind from core's labels", () => {
    expect(describe("/k/pods", "c").title).toBe("Pods");
    expect(describe("/k/pods", "c").kind).toBe("workloads");
  });

  it("titles a custom resource from its slug, as a resource", () => {
    const info = describe("/k/certificates.cert-manager.io", "c");
    expect(info.title).toBe("certificates.cert-manager.io");
    expect(info.kind).toBe("workloads");
  });

  it("names a resource detail after the resource", () => {
    expect(describe("/resources/web-1", "c")).toMatchObject({ title: "web-1", kind: "resource", sub: "c" });
  });

  it("names a /k/ resource detail after the resource, not its kind or slug", () => {
    // /k/Pod/default/web-1 shares a prefix with the /k/pods list route — the
    // detail branch must win, or this would title itself "Pod/default/web-1"
    // and carry kind "workloads" via the list-route fallback below.
    expect(describe("/k/Pod/default/web-1", "c")).toMatchObject({ title: "web-1", kind: "resource", sub: "c" });
  });

  it("names a cluster-scoped /k/ resource detail the same way", () => {
    expect(describe("/k/Node/-/worker-1", "c")).toMatchObject({ title: "worker-1", kind: "resource", sub: "c" });
  });

  it("titles the /resources/<name>/logs|shell|forward shapes distinctly from the bare resource tab", () => {
    // Same prefix as the bare resource route, so without a distinct title and
    // kind for each suffix, opening a pod three ways (Open in new tab, Follow
    // logs, Open shell) produced three indistinguishable tabs in the strip.
    // Only `shell` is still minted by the row menu; the other two are shapes a
    // session persisted before Logs and Port forward got real front doors, and
    // a restored tab still has to be able to name itself.
    expect(describe("/resources/web-1/logs", "c")).toMatchObject({ title: "web-1 · logs", kind: "logs", sub: "c" });
    expect(describe("/resources/web-1/shell", "c")).toMatchObject({
      title: "web-1 · shell",
      kind: "terminal",
      sub: "c",
    });
    expect(describe("/resources/web-1/forward", "c")).toMatchObject({
      title: "web-1 · forward",
      kind: "forwards",
      sub: "c",
    });
    // The bare route is unaffected — same title and kind as before.
    expect(describe("/resources/web-1", "c")).toMatchObject({ title: "web-1", kind: "resource", sub: "c" });
  });

  it("names a logs tab after its subject, not after the raw route", () => {
    // `/logs/<kind>/<namespace>/<name>` is four segments deep and matched by
    // none of the exact tables, so without this it fell to the last-resort
    // branch and titled itself "logs/Deployment/checkout/checkout-api" under
    // the control-room icon — the one screen on the strip whose tab could not
    // say which workload it was streaming.
    expect(describe(logsRoute("Deployment", "checkout", "checkout-api"), "c")).toMatchObject({
      title: "checkout-api · logs",
      kind: "logs",
      sub: "c",
    });
    // Percent-encoded on the way in, so the title is the decoded name.
    expect(describe(logsRoute("Pod", "kube-system", "weird/name"), "c").title).toBe("weird/name · logs");
    // The bare route keeps the plain title the sidebar's vocabulary gives it.
    expect(describe("/logs", "c")).toMatchObject({ title: "Logs", kind: "logs", sub: "c" });
  });

  it("names an edit tab after what it edits", () => {
    expect(describe("/edit/web-1", "c")).toMatchObject({ title: "Edit web-1", kind: "edit" });
  });

  it("falls back to the path for a route it has never heard of", () => {
    expect(describe("/whatever", "c")).toMatchObject({ title: "whatever", kind: "control" });
  });

  it("still describes a cluster-scoped route when there is no cluster yet", () => {
    // First launch with no contexts: the tab must still have a title.
    expect(describe("/k/pods").title).toBe("Pods");
    expect(describe("/k/pods").sub).toBeUndefined();
  });

  it("titles /k/events as Events, not its raw slug", () => {
    // events is in K8S_KIND, so isBuiltInKind("events") is true, but the
    // /k/ branch's RESOURCE_LABELS lookup must still resolve it — this pins
    // that against a regression, not against screenFor's routing.
    expect(describe("/k/events", "c")).toMatchObject({ title: "Events", kind: "workloads" });
  });
});

suite("screenFor", () => {
  it("resolves the screens that have been ported", () => {
    expect(screenFor("/applog")).toBe(AppLog);
    expect(screenFor("/notes")).toBe(ReleaseNotes);
  });

  it("resolves the tab strip's default cluster route to the Workloads union", () => {
    expect(screenFor("/resources")).toBe(Workloads);
  });

  it("resolves /events to the Events screen", () => {
    expect(screenFor("/events")).toBe(Events);
  });

  it("resolves /overview to the cluster overview the sidebar already points at", () => {
    // `lib/tree.ts`'s CLUSTER_ROUTES has mapped `overview` to `/overview`
    // since the sidebar was built, so without an entry here the first node in
    // the tree — the one a reader clicks before anything else — lands on the
    // Placeholder while the screen sits there finished.
    expect(screenFor("/overview")).toBe(Overview);
  });

  it("resolves both shapes of the logs route to the Logs screen", () => {
    // The bare route is the empty state that asks for a subject; the deep one
    // is a stream. One screen answers both, because `parseLogsRoute` is what
    // tells them apart and it lives inside it.
    expect(screenFor("/logs")).toBe(Logs);
    expect(screenFor(logsRoute("Deployment", "checkout", "checkout-api"))).toBe(Logs);
    expect(screenFor(logsRoute("Pod", "kube-system", "weird/name"))).toBe(Logs);
  });

  it("resolves /forwards to the port forwards screen", () => {
    // The route has parsed to `kind: "forwards"` since the tab strip was
    // built; without an entry here it rendered the Placeholder while the
    // screen sat there finished.
    expect(screenFor("/forwards")).toBe(Forwards);
  });

  it("resolves /toolbox to the toolbox screen", () => {
    // App-scoped, not cluster-scoped: the managed tools are the machine's,
    // not any one cluster's.
    expect(screenFor("/toolbox")).toBe(Toolbox);
  });

  it("leaves the retired /resources/<name>/forward shape on the Placeholder", () => {
    // NOTHING MINTS THIS ANY MORE. `ResourceMenu.tsx`'s `Port forward` used to,
    // and it carried neither a namespace nor a port — so it could not name a
    // tunnel, and routing it to Forwards would have opened the whole-cluster
    // list under a title claiming one resource. That entry opens §A.4's dialog
    // on the row instead, so the only way to arrive here now is a tab
    // persisted by an older session. It still parses (the strip has to be able
    // to title a restored tab) and it still renders the Placeholder rather
    // than the wrong screen.
    expect(screenFor("/resources/web-1/forward")).toBeNull();
  });

  it("leaves the row menu's older /resources/<name>/logs shape on the Placeholder", () => {
    // `ResourceMenu.tsx` still mints this shape, and it carries neither a kind
    // nor a namespace — so it cannot resolve a subject and must NOT be routed
    // to Logs, which would strand the reader on an unresolvable stream instead
    // of the Placeholder that says the screen is not wired up. Reconciling the
    // two shapes is its own step; this pins the dead end as a known one.
    expect(screenFor("/resources/web-1/logs")).toBeNull();
  });

  it("resolves /k/events to the Events screen rather than the generic resource list", () => {
    // events is in core's K8S_KIND, so isBuiltInKind("events") is true and,
    // absent a special case, the /k/ prefix loop would hand it to Resources —
    // whose descriptors.ts TYPED table has no "events" entry, so it would
    // fall to the generic three-column (Name/Namespace/Age) descriptor and
    // silently lose Type, Reason, Object, Message and Count.
    expect(screenFor("/k/events")).toBe(Events);
  });

  it("gives a route with no screen a placeholder", () => {
    for (const route of ["/", "/settings"]) {
      expect(screenFor(route), route).toBeNull();
    }
  });

  it("resolves a built-in kind's list route", () => {
    expect(screenFor("/k/pods")).toBe(Resources);
    expect(screenFor("/k/deployments")).toBe(Resources);
  });

  it("resolves a custom resource's list route", () => {
    // One screen answers every `/k/` route, so a slug nobody enumerated —
    // a CRD this cluster happens to have — reaches the same one.
    expect(screenFor("/k/widgets.example.com")).toBe(Resources);
  });

  it("resolves a detail route to the detail screen, not the list screen they share a prefix with", () => {
    // /k/pods (a list) and /k/Pod/default/web-1 (a detail) share the "/k/"
    // prefix. Get this wrong and every detail route in the app renders the
    // list screen as if the resource's name were just another kind slug.
    expect(screenFor("/k/pods")).toBe(Resources);
    expect(screenFor("/k/Pod/default/web-1")).toBe(ResourceDetailScreen);
    // The cluster-scoped sentinel is a namespace segment like any other, so a
    // Node's route resolves the same way a Pod's does.
    expect(screenFor("/k/Node/-/worker-1")).toBe(ResourceDetailScreen);
  });

  it("keeps a custom resource's detail route on the detail screen", () => {
    // A CRD's kind can be anything, including something that reads like a
    // slug — the segment COUNT is what tells a detail route from a list one.
    expect(screenFor("/k/Widget/default/left")).toBe(ResourceDetailScreen);
    expect(screenFor("/k/widgets.example.com")).toBe(Resources);
  });

  it("resolves a detail route whose name needed encoding", () => {
    // `detailRoute` percent-encodes every segment; a name with a slash in it
    // would otherwise split into six and match nothing at all.
    expect(screenFor("/k/Pod/default/web%2F1")).toBe(ResourceDetailScreen);
  });

  it("still refuses a route with no screen", () => {
    // `/k/` on its own names no kind, so it is not a route: a prefix that
    // matched itself would render the list screen with an empty slug.
    for (const route of ["/topology", "/k/", "/logs/", "constructor"]) {
      expect(screenFor(route), route).toBeNull();
    }
  });

  it("does not hand out Object.prototype members as screens", () => {
    // A route is a string from a tab, which can come from a persisted session
    // or a resource name. `SCREENS["constructor"]` on a plain object literal is
    // a function, so `Body` would have rendered `Object` as a component.
    for (const route of ["constructor", "/constructor", "toString", "__proto__", "hasOwnProperty"]) {
      expect(screenFor(route), route).toBeNull();
    }
  });
});

suite("describe against inherited keys", () => {
  it("falls through to the default title rather than a prototype member", () => {
    // Same hole as `screenFor`: `APP_SCOPED["constructor"]` on a plain object
    // literal is truthy, and spreading a function produced a title-less tab.
    expect(describe("constructor", "c")).toMatchObject({ title: "constructor", kind: "control" });
    expect(describe("/constructor", "c")).toMatchObject({ title: "constructor", kind: "control" });
    expect(describe("toString", "c").title).toBe("toString");
  });
});
