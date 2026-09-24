/**
 * Test-only: a `watchNamespaces` stand-in built on a suite's `watchResource`
 * mock, so a screen suite keeps stubbing one namespace's watch at a time and
 * still sees exactly which namespaces the screen asked for.
 *
 * Why a stand-in at all: mocking `watchResource` on `@srelens/core` does not
 * reach core's own `watchNamespaces`, which calls its module-local
 * `watchResource`. The real fan-out (merge order, hold-until-settled,
 * aggregate status) is tested in core's `watch.test.ts`; this only has to
 * route each namespace to the suite's mock and merge what comes back.
 */
type Rows = Array<{ name: string; namespace?: string }>;
type WatchResourceMock = (
  context: string,
  namespace: string,
  kind: string,
  onRows: (rows: Rows) => void,
  onStatus?: (status: "live" | "reconnecting") => void,
  onError?: (error: string) => void,
  files?: string[],
) => Promise<{ stop: () => void }>;

export function watchNamespacesVia(watchResource: WatchResourceMock) {
  return async (
    context: string,
    selection: string[],
    kind: string,
    onRows: (rows: Rows) => void,
    onStatus?: (status: "live" | "reconnecting") => void,
    onError?: (error: string, namespace: string) => void,
    files: string[] = [],
  ) => {
    const scopes = selection.length === 0 ? [""] : [...new Set(selection)];
    const snapshots = new Map<string, Rows>();
    const handles = await Promise.all(
      scopes.map((ns) =>
        watchResource(
          context,
          ns,
          kind,
          (rows) => {
            snapshots.set(ns, rows);
            onRows([...snapshots.values()].flat());
          },
          onStatus,
          (error) => onError?.(error, ns),
          files,
        ),
      ),
    );
    return { stop: () => handles.forEach((h) => h.stop()) };
  };
}
