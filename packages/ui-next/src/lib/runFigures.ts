import type { Turn } from "./agentRun";

/**
 * What a run's own turns can say about it (#386): how many tool calls it made,
 * and how long srelens spent answering.
 *
 * Answering time is the sum, over each question answered, of the answer's `at`
 * minus the question's. `askAgent` re-stamps an answer's `at` the moment it
 * settles; a question keeps the moment it was sent. A pair either side of which
 * carries no time of its own (`atRecorded === false`) says nothing and is
 * skipped, as is the answer still in flight. `null` is "nothing could be
 * measured" — not zero seconds.
 */
export function runFigures(turns: readonly Turn[], busy: boolean): { calls: number; answeringMs: number | null } {
  let calls = 0;
  let answeringMs: number | null = null;
  turns.forEach((turn, i) => {
    calls += turn.calls.length;
    const question = turns[i - 1];
    if (turn.role === "user" || question?.role !== "user") return;
    if (busy && i === turns.length - 1) return;
    if (turn.atRecorded === false || question.atRecorded === false) return;
    const ms = turn.at - question.at;
    if (ms < 0) return;
    answeringMs = (answeringMs ?? 0) + ms;
  });
  return { calls, answeringMs };
}

/** `11.2s` under a minute — the mock's own precision — then the whole-second
 *  steps core's `durationBetween` uses everywhere else (`2m 5s`, `1h 3m`). */
export function formatAnswering(ms: number): string {
  if (ms < 60_000) return `${(Math.round(ms / 100) / 10).toFixed(1)}s`;
  const secs = Math.floor(ms / 1000);
  const mins = Math.floor(secs / 60);
  const remSecs = secs % 60;
  if (mins < 60) return remSecs ? `${mins}m ${remSecs}s` : `${mins}m`;
  const hours = Math.floor(mins / 60);
  const remMins = mins % 60;
  return remMins ? `${hours}h ${remMins}m` : `${hours}h`;
}
