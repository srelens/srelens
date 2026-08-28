import { ageFromTimestamp, asRecord, durationBetween, str, timestampWithAge, type K8sObject } from "@srelens/core";
import { KV } from "@srelens/ui-kit";
import { Section } from "./Section";

/**
 * A Job's run — classic's "Job" section, ported fact-for-fact: completions,
 * parallelism, how many pods succeeded/failed/are still active, when it
 * started and completed, and its run time. Duration comes from core's
 * `durationBetween` once the Job has completed; a Job still running shows
 * its elapsed time instead, suffixed "(running)" the way classic does,
 * since there is no end timestamp yet to measure a duration against.
 */
export function JobDetailsBody({ object }: { object: K8sObject }) {
  const spec = asRecord(object.spec);
  const status = asRecord(object.status);
  const startTime = str(status.startTime);
  const completionTime = str(status.completionTime);
  const now = Date.now();
  const duration = completionTime
    ? durationBetween(startTime, completionTime)
    : startTime
      ? `${ageFromTimestamp(startTime, now)} (running)`
      : "—";

  return (
    <Section title="Job">
      <KV k="Completions" v={str(spec.completions)} />
      <KV k="Parallelism" v={str(spec.parallelism)} />
      <KV k="Succeeded" v={str(status.succeeded) || "0"} />
      <KV k="Failed" v={str(status.failed) || "0"} />
      <KV k="Active" v={str(status.active) || "0"} />
      <KV k="Started" v={startTime ? timestampWithAge(startTime, now) : "—"} />
      <KV k="Completed" v={completionTime ? timestampWithAge(completionTime, now) : "—"} />
      <KV k="Duration" v={duration} />
    </Section>
  );
}
