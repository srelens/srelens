import {
  containerVerdict,
  containersReadyText,
  describeContainer,
  type PodContainer,
} from "@srelens/core";

/**
 * How many blocks a row draws before the rest become `+N`. A pod with a
 * dozen containers is real, and a column that grew with them would push every
 * other column of every other row along with it.
 */
export const MAX_CONTAINER_BLOCKS = 8;

/**
 * A pod's containers, one small square each, in the Pods list (#878).
 *
 * The column this replaces printed `1/2`: how many containers are ready, and
 * not which one is not, or why — so the answer was always one click away, on
 * every row. A square per container puts it on the row: which container, in
 * what state, with its name and reason a hover away.
 *
 * What each square looks like is `.ctr-block` in `next.css`, keyed on the
 * state `containerVerdict` gives — core's one reading of a container, so this
 * list cannot disagree with a pod's own page about what a container is doing.
 * Shape carries the state as well as colour does (filled, outlined, dashed),
 * because colour alone is no signal to a reader who cannot tell the green
 * from the orange.
 *
 * **A restart is a mark on the square, not a square of its own.** A container
 * that has restarted is also running, or also in a back-off, right now; the
 * square says which, and a dot on its corner says it has been round before.
 *
 * Init containers stand apart, after a gap and a size smaller: they are not
 * what the pod runs, and one that has finished is the ordinary case. A
 * sidecar is drawn with the app containers, since it runs beside them for the
 * pod's whole life.
 *
 * The count the old column printed is still here — as the group's name, for a
 * screen reader, and in its tooltip.
 */
export function ContainerBlocks({
  containers,
  fallback,
}: {
  containers: readonly PodContainer[] | undefined;
  /** What to print when the row carries no per-container state at all: the
   *  ready count it always had. */
  fallback?: string;
}) {
  if (!containers || containers.length === 0) return <>{fallback ?? "—"}</>;
  const shown = containers.slice(0, MAX_CONTAINER_BLOCKS);
  const hidden = containers.slice(MAX_CONTAINER_BLOCKS);
  const main = shown.filter((c) => c.kind !== "init");
  const init = shown.filter((c) => c.kind === "init");
  const summary = containersReadyText(containers);
  return (
    <span role="group" aria-label={summary} title={summary} className="ctr-blocks">
      {main.map((c) => (
        <Block key={`${c.kind}/${c.name}`} container={c} />
      ))}
      {init.length > 0 && (
        <span className="ctr-blocks-init">
          {init.map((c) => (
            <Block key={`${c.kind}/${c.name}`} container={c} />
          ))}
        </span>
      )}
      {hidden.length > 0 && (
        <span className="ctr-more" title={hidden.map(describeContainer).join("\n")}>
          +{hidden.length}
        </span>
      )}
    </span>
  );
}

function Block({ container }: { container: PodContainer }) {
  const verdict = containerVerdict(container);
  const says = describeContainer(container);
  return (
    <span
      role="img"
      aria-label={says}
      title={says}
      className="ctr-block"
      data-state={verdict.kind}
      data-kind={container.kind}
      data-restarted={verdict.restarted || undefined}
    />
  );
}
