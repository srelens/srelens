# The Extension Inspector and per-app logs

What an installed app is doing now, why it failed, and what it costs
([#575](https://github.com/srelens/srelens/issues/575)). Both live in
**Settings → Apps → app → Details**, as two tabs beside **Overview** (the
manifest, grants, source, settings and previous versions of
[#534](https://github.com/srelens/srelens/issues/534)).

| Piece | Code |
|---|---|
| One app's log: levels, bounds, redaction | `crates/plugin-host/src/app_log.rs`, `app_log/redact.rs` |
| What the Inspector reads from a sidecar | `crates/plugin-host/src/sidecar/metrics.rs` (`Inspect`, `SidecarMetrics`) |
| The apps' logs and sidecars in this process, and the two capabilities | `crates/registry/src/extensions/inspector.rs` (`AppRuntime`) |
| The tabs | `packages/ui-next/src/extensions/ExtensionInspector.tsx`, `ExtensionLogs.tsx` |

## Inspector

| Section | From |
|---|---|
| ID, version, extension API version, revision, source and signature, grants | the inventory (`extensions.list`) |
| Active contributions and registered capabilities | the manifest; active only while the app is enabled, not quarantined and not blocked by policy |
| Process: state, reason, PID, negotiated sidecar API version, launches, unexpected exits | `extensions.inspect` → the supervisor's `SidecarStatus` |
| Memory, and who enforces its limit | the sandbox backend: the cgroup's `memory.current` on Linux; the process's committed private memory on Windows, the measure its Job Object's limit caps; the physical footprint on macOS, its watchdog's latest reading, which it takes every 50 ms and not while it holds the sidecar paused for its CPU limit (a stopped process allocates nothing) ([#713](https://github.com/srelens/srelens/issues/713)) |
| Requests: answered, failed, timed out, refused, in flight; latency p50, p95, max | the supervisor, over its last 256 answers |
| Open streams and watches | the app streams its views opened ([streams.md](streams.md#metrics)), watches apart from the rest, and the sidecar's own streams |
| Recent errors | the app's log: its last 20 errors, kept apart from the rest |

A **declarative app has no process**. The Inspector says so instead of showing
empty numbers, and its log says that nothing writes to it. Its pages and cards
show their own failures where they happen.

When a sidecar has been disabled after its restarts run out
([sidecar-protocol.md](sidecar-protocol.md#restart-backoff)), the Inspector
shows the supervisor's headline, **Extension process exited unexpectedly**,
with the last exit's reason and the supervisor's actions:

- **View logs** opens the Logs tab.
- **Disable** turns the app off.
- **Restart** is not offered yet. No host capability restarts a sidecar until
  executable apps are wired to the registry ([#574](https://github.com/srelens/srelens/issues/574)).

No app runs a process today: the manifest has no executable kind yet (#574).
When #574 starts a sidecar it asks `AppRuntime::log(id)` for the app's log,
passes that log to `Supervisor::start_with_log`, and shows the process with
`AppRuntime::attach(id, supervisor)`. The Inspector reads the supervisor only
through `Inspect::metrics`.

## Logs

Each app has its own log, separate from srelens's own log. Neither ever
receives a line of the other.

- **What goes in:** what the app's sidecar writes to stderr, and what srelens
  does about the app: each start, exit, restart and refusal, and each failed
  request.
- **Levels:** trace, debug, info, warn and error. A sidecar line that starts
  with a level word is kept at that level, and the word is removed
  ([sidecar-protocol.md](sidecar-protocol.md#logs)). srelens writes each start
  and stop at info, each restart and each refusal at a limit at warn, and each
  exit and failed request at error.
- **Bounds:** the last 1,000 lines, plus the last 20 errors kept apart. Dropped
  lines are counted, and the Logs tab says how many. A sidecar line longer than
  4 KiB is dropped, with a warning that it was. A line srelens writes is cut at
  4 KiB, as is a line that redaction made longer.
- **Reading:** `extensions.logs` answers the lines after the last one a reader
  has (`after`), at a level or above (`minLevel`). The Logs tab asks again
  every 2 seconds while it is open.

### Redaction

Every line is redacted before it enters the buffer. `AppLog::push` is the only
way in, so no reader can get a line that skipped redaction. The same rules apply
to a sidecar's state reason, which can quote what the sidecar said.

- **Values the host knows** (`AppLog::scrub`) are removed wherever they appear,
  verbatim or JSON-escaped. This is the guarantee: a value srelens knows is
  secret never reaches the buffer.
- **Text shaped like a credential** is removed by pattern:
  - `Authorization`, `Cookie` and API-key headers;
  - bearer and basic credentials;
  - a URL's user and password;
  - a `key=value` whose key names a credential (`token`, `password`,
    `client-key-data`, `sig`, …);
  - JWTs, including Kubernetes service account tokens;
  - PEM private keys, including one written over several lines;
  - the prefixes of common API tokens.

  This is best effort. No pattern recognizes every secret in free text, and a
  sidecar that wants to hide one in its own log can. The patterns catch a
  credential written by mistake.

Over-redacting is the safe direction. `author=` loses its value too.

## Local only

The log and the metrics live in the srelens process's memory, bounded, and go
when it exits:

- **Not on disk.** Nothing here is written to a file.
  `nothing_the_inspector_reads_is_written_to_disk` checks every file under the
  inventory's directory.
- **Not to an agent.** `extensions.inspect` and `extensions.logs` are
  **UI-only** (`Capability::ui_only`). `McpServer::new` drops them, so no MCP
  path can list or call them, and `docs/mcp-catalog.md` leaves them out. There
  are two reasons: an agent's context goes to its LLM provider, and a sidecar's
  stderr is text a third party wrote, which would be a way to inject
  instructions into the agent.
  `app_logs_and_metrics_never_leave_through_mcp_or_the_audit_trail` checks that
  these two are the only UI-only capabilities and that MCP refuses them on
  both transports.
- **Not in the audit trail**, and srelens has no telemetry
  ([SECURITY.md](../../.github/SECURITY.md)).
- **No network client.** A test fails if the modules that hold the log and
  metrics name one.

On the web host, the browser reads both capabilities over `/api`, as it reads
every other screen. That is srelens's own UI, not a transmission.
