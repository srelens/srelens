#!/usr/bin/env node
// Launch srelens in web mode and screenshot one route, end to end.
//
//   node scripts/screenshot.mjs <route> <out.png> [flags]
//
//   node scripts/screenshot.mjs / .screenshots/home.png
//   node scripts/screenshot.mjs '#gallery' .screenshots/gallery.png
//   node scripts/screenshot.mjs /events .screenshots/events.png
//   node scripts/screenshot.mjs /k/pods .screenshots/pods.png
//
// Flags:
//   --width=N --height=N     viewport (default 1600x1000)
//   --design=next|classic    which design to render (default next)
//   --context=NAME           kube context to select (default: the kubeconfig's
//                            own current-context, i.e. what kubectl would use)
//   --scroll-to=TEXT         scroll the heading whose text is TEXT into view
//   --click=TEXT             click the button/control whose text is TEXT, then
//                            settle again (repeatable: --click=A --click=B)
//   --select=LABEL:VALUE     set the <select> whose aria-label is LABEL to
//                            VALUE, e.g. --select=Since:all
//   --wait=MS                extra settle time after the app stops loading
//   --build / --no-build     force or skip the frontend rebuild (default: rebuild
//                            when a source file is newer than apps/desktop/dist)
//   --keep-server            leave the server running after the shot
//   --port=N                 server port (default 8781)
//
// WHAT IS NOT OBVIOUS — read this before changing anything here.
//
// 1. Web mode is a Rust binary (`crates/server`), not `vite preview`. The
//    frontend's `Developer login` button POSTs `/auth/dev-login`, which only
//    that binary serves, so a statically served build can never get past the
//    login screen. The binary is `srelens-server serve <addr> --data <dir>`.
//
// 2. Dev login is gated on an env var, not a feature flag:
//    `SRELENS_DEV_LOGIN=<email>` enables `POST /auth/dev-login`
//    (crates/server/src/auth/mod.rs). Without it — or without OIDC — the
//    server refuses to start at all. `SRELENS_MASTER_KEY` (64 hex chars) is
//    also mandatory; it seals stored kubeconfigs at rest, and it must stay
//    STABLE across runs or the kubeconfig uploaded by an earlier run becomes
//    undecryptable. Hence the fixed throwaway key below.
//
// 3. `POST /auth/dev-login` needs no CSRF header and no body. It replies 302
//    with `Set-Cookie: srelens_session=…; HttpOnly`. HttpOnly means the page
//    cannot plant it, so the cookie goes into the browser over the DevTools
//    Protocol (`Network.setCookie`) instead. That is why this drives Chrome
//    over CDP rather than just passing it a URL: the same connection also
//    seeds localStorage, which is the only way to pick a design and a route
//    (see 4 and 5).
//
// 4. Which design renders is `localStorage["srelens.design"]`, read
//    synchronously before React mounts (apps/desktop/src/design.ts). It has
//    to be written and the page reloaded; there is no URL or query for it.
//
// 5. THE NEW DESIGN HAS NO URL ROUTER. `/events`, `/k/pods` and `/overview`
//    are *tab* routes inside an in-memory store, and the server's SPA
//    fallback serves index.html for every path, so navigating to
//    http://host/k/pods just opens the default tab. The tab set is persisted
//    to `localStorage["srelens.next.workspaces"]`
//    (packages/ui-next/src/lib/tabsPersist.ts) — and, importantly, a clean
//    boot never writes that document at all (nothing changes, so nothing is
//    saved), so there is no document to edit. This script builds it itself —
//    one workspace holding every context, a pinned home tab, and a tab at the
//    wanted route — and writes it
//    before the app's first boot. The cluster ids in it are the backend's
//    `stableId`s (`<kubeconfig path>#<context name>`), fetched from the same
//    `k8s.listContexts` capability the shell calls, because they are assigned
//    by the backend and cannot be guessed.
//    The gallery is the exception: it is `#gallery`, a hash, not a route
//    (packages/ui-next/src/index.tsx) — there is no `/components` route.
//
// 6. Web mode does NOT read ~/.kube/config. Every user uploads their own
//    kubeconfig, sealed in the server's SQLite DB (Settings → Kubernetes in
//    the UI, `POST /api/kubeconfigs` underneath). So to see a real cluster
//    this script uploads $KUBECONFIG (or ~/.kube/config) once, on the first
//    run against a given data dir. `/api/*` requires BOTH the session cookie
//    and a non-empty `X-Srelens-Csrf` header.
//
// The server data dir lives in the OS temp dir, deliberately NOT in the repo:
// it holds an encrypted copy of your kubeconfig, and neither `.screenshots/`
// nor a repo-local data dir is gitignored.

import { spawn, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, openSync, readdirSync, readFileSync, statSync, writeFileSync, rmSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

// ---------------------------------------------------------------- arguments

const argv = process.argv.slice(2);
const flags = Object.fromEntries(
  argv
    .filter((a) => a.startsWith("--"))
    .map((a) => {
      const [k, v] = a.slice(2).split("=");
      return [k, v ?? true];
    }),
);
const positional = argv.filter((a) => !a.startsWith("--"));
const route = positional[0];
const outPath = positional[1] && path.resolve(positional[1]);
if (!route || !outPath) {
  console.error("usage: node scripts/screenshot.mjs <route> <out.png> [--width=1600] [--height=1000] [--design=next] [--scroll-to=TEXT] [--wait=MS] [--build] [--keep-server]");
  process.exit(2);
}

const PORT = Number(flags.port ?? 8781);
const ORIGIN = `http://127.0.0.1:${PORT}`;
const WIDTH = Number(flags.width ?? 1600);
const HEIGHT = Number(flags.height ?? 1000);
const DESIGN = String(flags.design ?? "next");
const SETTLE_MS = Number(flags.wait ?? 3000);

// Local, throwaway, and deliberately constant: a fresh key each run would make
// the kubeconfig stored by the previous run unreadable (see note 2).
const MASTER_KEY = "5c3e1b7a9d0f4628a1b3c5d7e9f0123456789abcdef0123456789abcdef01234";
const DEV_EMAIL = "screenshot@localhost";
const DATA_DIR = path.join(tmpdir(), "srelens-screenshot-data");
const SERVER_BIN = path.join(ROOT, "target", "debug", "srelens-server");
const CHROME = process.env.CHROME ?? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function run(cmd, args, opts = {}) {
  const res = spawnSync(cmd, args, { cwd: ROOT, stdio: "inherit", ...opts });
  if (res.status !== 0) throw new Error(`${cmd} ${args.join(" ")} failed (${res.status})`);
}

// --------------------------------------------------------------- build step

// The server serves `apps/desktop/dist`, so a screenshot only shows the last
// build. Rebuilt automatically when a source file is newer than the build —
// otherwise a re-run after a change quietly photographs the previous one.
function newestMtime(dir) {
  let newest = 0;
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (entry.name === "node_modules" || entry.name === "dist" || entry.name.startsWith(".")) continue;
    const full = path.join(dir, entry.name);
    newest = Math.max(newest, entry.isDirectory() ? newestMtime(full) : statSync(full).mtimeMs);
  }
  return newest;
}
const dist = path.join(ROOT, "apps/desktop/dist/index.html");
const stale =
  !existsSync(dist) ||
  ["apps/desktop/src", "packages"].some((d) => newestMtime(path.join(ROOT, d)) > statSync(dist).mtimeMs);
if (!flags["no-build"] && (flags.build || stale)) {
  console.error("· building the frontend…");
  run("pnpm", ["--filter", "@srelens/desktop", "build"]);
}
// The BACKEND goes stale the same way the frontend does, and far more quietly.
// Existence alone is not enough: a binary built before a new capability was
// registered answers `capability not found` for it, and the screen photographs
// as a page of errors and NaNs that look exactly like a bug in the screen. That
// happened — a whole overview shot with `k8s.clusterFacts` missing and every
// meter reading NaN%, against code where all of it worked. Cargo would no-op a
// fresh build in seconds, so the mtime check only saves the link step.
const serverStale =
  !existsSync(SERVER_BIN) ||
  newestMtime(path.join(ROOT, "crates")) > statSync(SERVER_BIN).mtimeMs;
if (serverStale) {
  // Debug build on purpose: rust-embed reads apps/desktop/dist from disk in a
  // debug build and bakes it into the binary in a release one, so a debug
  // server picks up a frontend rebuild with no recompile.
  console.error("· building srelens-server (debug; a few minutes the first time)…");
  run("cargo", ["build", "-p", "srelens-server", "--bin", "srelens-server"]);
}

// -------------------------------------------------------------- the server

async function serverUp() {
  try {
    const res = await fetch(`${ORIGIN}/healthz`, { signal: AbortSignal.timeout(500) });
    return res.ok;
  } catch {
    return false;
  }
}

let server = null;
const startedServer = !(await serverUp());
if (startedServer) {
  mkdirSync(DATA_DIR, { recursive: true });
  console.error(`· starting srelens-server on ${ORIGIN} (data: ${DATA_DIR})`);
  // Its output goes to a file, not to our stdio: inheriting keeps the parent's
  // pipe open, so a `node scripts/screenshot.mjs … | tail` never terminates
  // when the server is left running.
  const log = openSync(path.join(DATA_DIR, "server.log"), "a");
  server = spawn(SERVER_BIN, ["serve", `127.0.0.1:${PORT}`, "--data", DATA_DIR], {
    cwd: ROOT,
    stdio: ["ignore", log, log],
    env: {
      ...process.env,
      SRELENS_DEV_LOGIN: DEV_EMAIL,
      SRELENS_MASTER_KEY: MASTER_KEY,
      SRELENS_PUBLIC_URL: ORIGIN,
    },
  });
  for (let i = 0; i < 100 && !(await serverUp()); i++) await sleep(100);
  if (!(await serverUp())) throw new Error("server did not come up");
} else {
  console.error(`· reusing the server already listening on ${ORIGIN}`);
}

// ----------------------------------------------------------------- log in

console.error("· POST /auth/dev-login");
const login = await fetch(`${ORIGIN}/auth/dev-login`, { method: "POST", redirect: "manual" });
const setCookie = login.headers.getSetCookie?.() ?? [login.headers.get("set-cookie")];
const session = setCookie
  .filter(Boolean)
  .map((c) => /(?:^|;\s*)srelens_session=([^;]+)/.exec(c)?.[1])
  .find(Boolean);
if (!session) {
  throw new Error(`no session cookie from /auth/dev-login (status ${login.status}) — is SRELENS_DEV_LOGIN set on the running server?`);
}

// /api/* wants the session cookie AND any non-empty X-Srelens-Csrf header.
const api = (p, init = {}) =>
  fetch(`${ORIGIN}${p}`, {
    ...init,
    headers: {
      cookie: `srelens_session=${session}`,
      "x-srelens-csrf": "1",
      "content-type": "application/json",
      ...(init.headers ?? {}),
    },
  });

// ------------------------------------------------------- kubeconfig upload

const kubeconfigPath = process.env.KUBECONFIG?.split(":")[0] ?? path.join(homedir(), ".kube", "config");
const kubeconfigYaml = existsSync(kubeconfigPath) ? readFileSync(kubeconfigPath, "utf8") : null;

const existing = await api("/api/kubeconfigs").then((r) => r.json());
if ((!Array.isArray(existing) || existing.length === 0) && kubeconfigYaml) {
  console.error(`· uploading ${kubeconfigPath} (web mode has no filesystem access to it)`);
  const res = await api("/api/kubeconfigs", {
    method: "POST",
    body: JSON.stringify({ name: "local", yaml: kubeconfigYaml }),
  });
  if (!res.ok) console.error(`  ! upload failed: ${res.status} ${await res.text()}`);
} else if (!kubeconfigYaml) {
  console.error(`  ! no kubeconfig at ${kubeconfigPath}; the app will render its no-cluster states`);
}

// Which context to look at. `current-context` is read straight out of the
// kubeconfig text rather than shelled out to kubectl, so this works with no
// kubectl on PATH; the app itself never picks a context, it just takes the
// first one it is handed (`defaultState` in packages/ui-next/src/lib/tabs.ts).
const wantContext =
  (typeof flags.context === "string" && flags.context) ||
  /^current-context:\s*"?([^"\n]+)"?\s*$/m.exec(kubeconfigYaml ?? "")?.[1]?.trim() ||
  null;

// The shell's own context list — this is where `stableId` comes from.
const contexts = await api("/api/capability/k8s.listContexts", {
  method: "POST",
  body: JSON.stringify({ paths: [] }),
})
  .then((r) => r.json())
  .then((j) => j.contexts ?? [])
  .catch(() => []);
if (contexts.length === 0) console.error("  ! the server reports no kube contexts");
const activeCluster =
  contexts.find((c) => c.name === wantContext)?.stableId ?? contexts[0]?.stableId ?? null;
if (wantContext && !contexts.some((c) => c.name === wantContext)) {
  console.error(`  ! no context called ${wantContext}; falling back to ${activeCluster ?? "none"}`);
}
console.error(`· cluster: ${activeCluster ?? "(none)"}`);

// -------------------------------------------------------------- the browser

// One fixed profile per port, wiped up front rather than a fresh temp dir each
// run: a run always starts from a browser that remembers nothing (so the
// localStorage seeded below is the whole truth), and there is only ever one of
// these to clean up. Chrome is still writing to it when we kill it, so it is
// cleared on the way IN, not on the way out.
const profile = path.join(tmpdir(), `srelens-shot-profile-${PORT}`);
rmSync(profile, { recursive: true, force: true });
mkdirSync(profile, { recursive: true });
const CDP_PORT = PORT + 1000;
const chrome = spawn(
  CHROME,
  [
    "--headless=new",
    "--disable-gpu",
    "--hide-scrollbars",
    `--remote-debugging-port=${CDP_PORT}`,
    `--user-data-dir=${profile}`,
    `--window-size=${WIDTH},${HEIGHT}`,
    "--no-first-run",
    "--no-default-browser-check",
    "about:blank",
  ],
  { stdio: ["ignore", "ignore", "ignore"] },
);

function cleanup() {
  try { chrome.kill(); } catch {}
  if (server && !flags["keep-server"]) {
    try { server.kill(); } catch {}
  } else if (server) {
    console.error(`· server left running on ${ORIGIN} (pid ${server.pid}) — kill it yourself`);
    server.unref();
  }
}
process.on("exit", cleanup);
process.on("SIGINT", () => process.exit(130));

// A minimal CDP client. Node 22 has a global WebSocket, so no dependency.
async function pageSocket() {
  for (let i = 0; i < 100; i++) {
    try {
      const targets = await fetch(`http://127.0.0.1:${CDP_PORT}/json/list`).then((r) => r.json());
      const page = targets.find((t) => t.type === "page" && t.webSocketDebuggerUrl);
      if (page) return page.webSocketDebuggerUrl;
    } catch {}
    await sleep(100);
  }
  throw new Error("chrome devtools never came up");
}

const ws = new WebSocket(await pageSocket());
await new Promise((resolve, reject) => {
  ws.addEventListener("open", resolve, { once: true });
  ws.addEventListener("error", reject, { once: true });
});

let nextId = 0;
const pending = new Map();
const events = [];
ws.addEventListener("message", (e) => {
  const msg = JSON.parse(e.data);
  if (msg.id !== undefined) {
    const p = pending.get(msg.id);
    pending.delete(msg.id);
    if (msg.error) p.reject(new Error(`${p.method}: ${msg.error.message}`));
    else p.resolve(msg.result);
  } else {
    events.push(msg.method);
  }
});

function cdp(method, params = {}) {
  const id = ++nextId;
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject, method });
    ws.send(JSON.stringify({ id, method, params }));
  });
}

async function evaluate(expression) {
  const res = await cdp("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true });
  if (res.exceptionDetails) throw new Error(res.exceptionDetails.exception?.description ?? "evaluate failed");
  return res.result.value;
}

/** Navigate and wait for the document to finish loading. */
async function go(url) {
  await cdp("Page.navigate", { url });
  for (let i = 0; i < 200; i++) {
    if ((await evaluate("document.readyState")) === "complete") return;
    await sleep(100);
  }
}

await cdp("Page.enable");
await cdp("Runtime.enable");
await cdp("Emulation.setDeviceMetricsOverride", {
  width: WIDTH,
  height: HEIGHT,
  deviceScaleFactor: 1,
  mobile: false,
});

// The session cookie is HttpOnly, so it can only be planted from out here.
await cdp("Network.enable");
await cdp("Network.setCookie", {
  name: "srelens_session",
  value: session,
  domain: "127.0.0.1",
  path: "/",
  httpOnly: true,
  sameSite: "Lax",
});

// Titles mirror `describe()` in packages/ui-next/src/lib/routes.ts: the strip
// renders whatever title the stored tab carries, so a wrong one here would show
// up in the screenshot.
const TITLES = {
  "/": ["Control room", "control"],
  "/overview": ["Cluster overview", "control"],
  "/events": ["Events", "events"],
  "/resources": ["Workloads", "workloads"],
  "/applog": ["Application log", "applog"],
  "/notes": ["Release notes", "notes"],
  "/logs": ["Logs", "logs"],
  "/helm": ["Helm", "helm"],
  "/forwards": ["Port forwards", "forwards"],
  "/toolbox": ["Toolbox", "toolbox"],
  "/topology": ["Topology", "topology"],
  "/incidents": ["Incidents", "incidents"],
};
function tabFor(r, id) {
  const slug = r.startsWith("/k/") ? r.slice(3) : null;
  // `/logs/<kind>/<namespace>/<name>` is titled after its subject, not after
  // its path — the same branch `describe()` grew when the screen was routed.
  // Without it a stream tab photographs as "logs/Deployment/payments/…".
  const logs = /^\/logs\/[^/]+\/[^/]+\/([^/]+)$/.exec(r);
  const [title, kind] = (logs ? [`${decodeURIComponent(logs[1])} · logs`, "logs"] : TITLES[r]) ?? [
    slug ? slug[0].toUpperCase() + slug.slice(1) : r.replace(/^\//, ""),
    slug ? "workloads" : "control",
  ];
  const tab = { id, route: r, title, kind };
  if (r === "/") tab.pinned = true;
  const name = contexts.find((c) => c.stableId === activeCluster)?.name;
  // App-scoped routes carry no cluster in their sub — `APP_SCOPED` in
  // packages/ui-next/src/lib/routes.ts is the list, and the toolbox is on it:
  // the managed tools are the machine's, not any one cluster's.
  if (name && !["/applog", "/notes", "/toolbox"].includes(r)) tab.sub = name;
  return tab;
}

const isHash = route.startsWith("#");
const wantTab = !isHash && route !== "/";
const workspace = {
  version: 1,
  currentId: "shot-ws",
  workspaces: [
    {
      id: "shot-ws",
      name: "Default",
      clusters: contexts.map((c) => c.stableId),
      tabs: [tabFor("/", "shot-home"), ...(wantTab ? [tabFor(route, "shot-route")] : [])],
      activeId: wantTab ? "shot-route" : "shot-home",
      closed: [],
      ...(activeCluster ? { activeCluster } : {}),
    },
  ],
};

// /healthz is text/plain, so this lands on the origin (which is what
// localStorage is keyed by) without booting the app first — the design and the
// workspace are in place before its one and only boot.
await go(`${ORIGIN}/healthz`);
await evaluate(`
  localStorage.setItem("srelens.design", ${JSON.stringify(DESIGN)});
  localStorage.setItem("srelens.next.workspaces", ${JSON.stringify(JSON.stringify(workspace))});
  localStorage.removeItem("srelens.restoreSession");
`);
await go(`${ORIGIN}/`);

/**
 * Wait until the shell has booted AND the screen has stopped saying it is
 * loading. Two waits, because they are two different things: the shell boots in
 * a few hundred ms, but a resource list then opens a watch over the WebSocket
 * and takes several seconds more to show a row. Screenshotting between the two
 * gets a spinner that looks like a working app.
 */
async function waitForApp() {
  const text = () => evaluate("document.getElementById('root')?.innerText ?? ''");
  for (let i = 0; i < 300; i++) {
    const t = await text();
    if (t && !/^\s*Loading\s*$/.test(t) && !t.includes("Checking session")) break;
    await sleep(100);
  }
  for (let i = 0; i < 200; i++) {
    if (!/Loading\b|Loading…/.test(await text())) break;
    await sleep(100);
  }
  await sleep(SETTLE_MS);
}
await waitForApp();

if (isHash) {
  // The gallery is a hash, and `hashchange` is what the app listens to, so the
  // hash is set in the live document rather than navigated to.
  await evaluate(`window.location.hash = ${JSON.stringify(route)}`);
  await sleep(1500);
}

// Some states are only reachable by pressing something — `Previous instance`
// on the logs screen is one: it is a toggle over the same stream, not a route,
// so no seeded workspace can open it. Matched on the control's own text rather
// than a selector, so this stays readable and does not encode a class name that
// the design owns.
const clicks = argv.filter((a) => a.startsWith("--click=")).map((a) => a.slice("--click=".length));
for (const wanted of clicks) {
  const found = await evaluate(`(() => {
    const controls = [...document.querySelectorAll("button,[role=button],[role=switch],[role=tab]")];
    const el = controls.find((c) => (c.innerText ?? "").trim() === ${JSON.stringify(wanted)})
      ?? controls.find((c) => (c.innerText ?? "").trim().includes(${JSON.stringify(wanted)}));
    if (!el) return false;
    el.scrollIntoView({ block: "center" });
    el.click();
    return true;
  })()`);
  if (!found) console.error(`  ! nothing to click called ${wanted}`);
  await waitForApp();
}

// A dropdown, for the states that hang off one — `Since` on the logs screen
// decides whether a stream opens with a container's whole history or with the
// last five minutes, and a workload that logs only at boot has nothing at all
// to show under the default. React listens for `change` on a <select>
// directly, so a plain dispatched event is enough here (an <input> would need
// the native value setter).
const selects = argv.filter((a) => a.startsWith("--select=")).map((a) => a.slice("--select=".length));
for (const pair of selects) {
  const at = pair.indexOf(":");
  const [label, value] = [pair.slice(0, at), pair.slice(at + 1)];
  const found = await evaluate(`(() => {
    const el = [...document.querySelectorAll("select")].find(
      (s) => (s.getAttribute("aria-label") ?? "").trim().toLowerCase() === ${JSON.stringify(label.toLowerCase())},
    );
    if (!el) return false;
    el.value = ${JSON.stringify(value)};
    el.dispatchEvent(new Event("change", { bubbles: true }));
    return true;
  })()`);
  if (!found) console.error(`  ! no select called ${label}`);
  await waitForApp();
}

if (flags["scroll-to"]) {
  const wanted = String(flags["scroll-to"]);
  const found = await evaluate(`(() => {
    const el = [...document.querySelectorAll("h1,h2,h3")].find((h) => h.textContent.trim() === ${JSON.stringify(wanted)});
    if (!el) return false;
    el.scrollIntoView({ block: "start" });
    return true;
  })()`);
  if (!found) console.error(`  ! no heading called ${wanted}`);
  await sleep(600);
}

const shot = await cdp("Page.captureScreenshot", { format: "png" });
mkdirSync(path.dirname(outPath), { recursive: true });
writeFileSync(outPath, Buffer.from(shot.data, "base64"));
console.error(`· wrote ${outPath}`);

ws.close();
process.exit(0);
