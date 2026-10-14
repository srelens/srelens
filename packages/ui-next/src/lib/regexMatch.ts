/**
 * What a user's `/regex/` is asked of a line (#898). This runs in the regex
 * worker and nowhere else: a pattern that backtracks catastrophically freezes
 * the thread it runs on, and a worker can be thrown away where the main thread
 * cannot. It has no `Worker` in it, so the worker does not import the module
 * that starts it.
 */

/** One line's question: the string the clause tests, and for a free-text regex the row as it is shown, to mark. */
export interface RegexItem {
  readonly test: string;
  readonly shown?: string;
}

/** Whether `test` matched, and where the pattern falls in `shown` when one was given. */
export interface RegexAnswer {
  readonly hit: boolean;
  readonly ranges?: readonly (readonly [number, number])[];
}

export function answer(source: string, flags: string, items: readonly RegexItem[]): RegexAnswer[] {
  const re = new RegExp(source, flags);
  const every = new RegExp(source, `${flags}g`);
  return items.map(({ test, shown }) => {
    const hit = re.test(test);
    if (shown === undefined) return { hit };
    const ranges: [number, number][] = [];
    every.lastIndex = 0;
    for (let m = every.exec(shown); m !== null; m = every.exec(shown)) {
      // An empty match would be found at the same place forever.
      if (m[0] === "") {
        every.lastIndex += 1;
        continue;
      }
      ranges.push([m.index, m.index + m[0].length]);
    }
    return { hit, ranges };
  });
}
