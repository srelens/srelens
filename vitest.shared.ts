/**
 * Settings every vitest project shares, in one place because each of the four
 * has to state them itself.
 *
 * The root `vitest.config.ts` owns `projects`, and a `test` option set in that
 * block does NOT reach them: a project config is resolved on its own, and only
 * the root-only options (coverage, reporters) are read from the root. Verified
 * by probe — a project test slept straight through a 50ms root budget, and
 * failed a 50ms project one. So the value lives here and each project imports
 * it, rather than four copies of a number and a paragraph drifting apart.
 */

/**
 * How long one test may take: 15s, not vitest's 5000ms default.
 *
 * Two tests failed CI on PR #380 purely on time — `AppLog > says how many
 * lines are shown when the cap truncates real matches` at 6070ms and
 * `Gallery > shows every component the kit exports` at 5240ms — while taking
 * 1322ms and 3213ms on a quiet developer machine. Neither test is slow. The
 * budget was simply never revisited while the suite grew to 5327 tests across
 * 349 files, all running in parallel on a shared CI runner where every worker
 * contends for the same cores.
 *
 * 15s is deliberately close: it clears the worst measured CI figure with
 * headroom and nothing more. A budget generous enough to sit through a genuine
 * hang is the thing being avoided.
 *
 * `hookTimeout` is deliberately NOT raised with it. Its own default is 10s
 * already, nothing in this suite's hooks comes near that, and no hook has ever
 * timed out in CI — the argument for the raise simply does not apply to them.
 */
export const TEST_TIMEOUT_MS = 15_000;
