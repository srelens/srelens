// Which containers of a pod a shell can actually be opened into, and which one
// to offer first (#262).
//
// `kubectl exec` with no `-c` picks the first container in the spec, so on a
// multi-container pod the shell button lands wherever the manifest happens to
// start — often a sidecar with no shell at all. Kubernetes' answer to that is
// the `kubectl.kubernetes.io/default-container` annotation, which kubectl
// honours client-side; the API server does not, so we resolve it here.

/** The annotation kubectl uses to pick a container when none is given. */
export const DEFAULT_CONTAINER_ANNOTATION = "kubectl.kubernetes.io/default-container";

/** A container the user could exec into. */
export interface ContainerChoice {
  name: string;
  /** Which section of the pod spec it came from. */
  kind: "app" | "init" | "ephemeral";
  /** Whether the container is currently running, i.e. exec can attach. */
  running: boolean;
}

function record(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}

function array(value: unknown): unknown[] {
  return Array.isArray(value) ? value : [];
}

function names(list: unknown): string[] {
  return array(list)
    .map((entry) => record(entry).name)
    .filter((name): name is string => typeof name === "string" && name !== "");
}

/** Names of containers reported as running in one of the status lists. */
function runningNames(status: Record<string, unknown>): Set<string> {
  const running = new Set<string>();
  for (const key of ["containerStatuses", "initContainerStatuses", "ephemeralContainerStatuses"]) {
    for (const entry of array(status[key])) {
      const s = record(entry);
      if (typeof s.name === "string" && "running" in record(s.state)) running.add(s.name);
    }
  }
  return running;
}

/**
 * Every container of `pod`, in menu order: app containers, then init
 * containers (native sidecars among them), then ephemeral debug containers.
 *
 * Containers that aren't running are listed too, marked rather than hidden. A
 * finished init container and a crash-looping app container are both things
 * the user is looking for when they open this menu, and a list that silently
 * omits half the pod is harder to trust than one that shows what is up.
 */
export function podContainerChoices(pod: unknown): ContainerChoice[] {
  const spec = record(record(pod).spec);
  const running = runningNames(record(record(pod).status));
  const of = (kind: ContainerChoice["kind"]) => (name: string) => ({
    name,
    kind,
    running: running.has(name),
  });
  return [
    ...names(spec.containers).map(of("app")),
    ...names(spec.initContainers).map(of("init")),
    ...names(spec.ephemeralContainers).map(of("ephemeral")),
  ];
}

/**
 * Containers worth asking the user about: the pod's own containers, plus any
 * init or ephemeral container still running.
 *
 * A finished init container is listed in the menu but doesn't count here — a
 * pod that is one app container and a completed migration step is not a choice,
 * and making the user pick every time would be noise.
 */
export function execCandidates(choices: readonly ContainerChoice[]): ContainerChoice[] {
  return choices.filter((c) => c.kind === "app" || c.running);
}

/**
 * The container to open by default: the one the pod's author nominated through
 * the annotation, else the first running app container, else the first listed.
 *
 * Preferring a running container over the spec's first entry matters on pods
 * whose leading container has crashed — the default should be somewhere a shell
 * can actually land.
 */
export function defaultContainer(pod: unknown, choices: ContainerChoice[]): string | undefined {
  const annotated = record(record(record(pod).metadata).annotations)[DEFAULT_CONTAINER_ANNOTATION];
  if (typeof annotated === "string" && choices.some((c) => c.name === annotated)) return annotated;
  const app = choices.filter((c) => c.kind === "app");
  return (app.find((c) => c.running) ?? app[0] ?? choices[0])?.name;
}
