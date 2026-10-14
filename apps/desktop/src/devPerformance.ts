/**
 * Empties the performance timeline on an interval, and returns the function
 * that stops it.
 *
 * React 19's development build records a `performance.measure` entry for every
 * component render and never clears them. A busy log stream renders ~1,300
 * times a second, so after a few minutes the dev renderer holds over 1 GB and
 * crashes. A production build records none of this. Only measures are cleared;
 * marks and everything else on the timeline are left alone.
 */
export function trimDevMeasures(every = 1000): () => void {
  const id = setInterval(() => performance.clearMeasures(), every);
  return () => clearInterval(id);
}
