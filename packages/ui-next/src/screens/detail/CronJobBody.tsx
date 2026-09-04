import {
  ageSortValue,
  asArray,
  asRecord,
  jobStatus,
  listJobs,
  str,
  timestampWithAge,
  type JobSummary,
  type K8sObject,
} from "@srelens/core";
import { KV, LoadingState, StatusPill, Table, type Column } from "@srelens/ui-kit";
import { Section } from "./Section";
import { SectionFailure, useSectionList } from "./sectionList";
import { StringList } from "./sections";
import { AgeCell } from "../../lib/ageCell";

/**
 * A CronJob's schedule — classic's "Schedule" section, ported fact-for-fact:
 * the cron expression, whether it's suspended, its concurrency policy, when
 * it last ran, how much history it keeps, and its currently-active Jobs.
 * "Active jobs" is a `LinkedResources` list in classic that navigates to
 * each Job; here it renders as inert "Kind/name" text — see the task report
 * for the full inert-value list.
 */
function ScheduleSection({ object }: { object: K8sObject }) {
  const meta = object.metadata ?? {};
  const spec = asRecord(object.spec);
  const status = asRecord(object.status);
  const namespace = str(meta.namespace) || null;
  const lastSchedule = str(status.lastScheduleTime);
  const successKept = str(spec.successfulJobsHistoryLimit) || "3";
  const failedKept = str(spec.failedJobsHistoryLimit) || "1";
  const activeJobs = asArray(status.active)
    .map(asRecord)
    .map((job) => ({
      kind: str(job.kind) || "Job",
      namespace: str(job.namespace) || namespace,
      name: str(job.name),
    }))
    .filter((job) => job.name);

  return (
    <Section title="Schedule">
      <KV k="Schedule" v={str(spec.schedule)} mono />
      <KV k="Suspend" v={spec.suspend === true ? "Yes" : "No"} />
      <KV k="Concurrency policy" v={str(spec.concurrencyPolicy)} />
      <KV k="Last schedule" v={lastSchedule ? timestampWithAge(lastSchedule, Date.now()) : "—"} />
      <KV k="History (kept)" v={`${successKept} succeeded, ${failedKept} failed`} />
      <KV
        k="Active jobs"
        v={
          activeJobs.length > 0 ? (
            <StringList items={activeJobs.map((j) => `${j.kind}/${j.name}`)} />
          ) : (
            "0"
          )
        }
      />
    </Section>
  );
}

const RECENT_JOB_COLUMNS: Column<JobSummary>[] = [
  { key: "name", header: "Name", render: (j) => <span className="font-mono">{j.name}</span> },
  { key: "completions", header: "Completions", render: (j) => j.completions },
  {
    key: "status",
    header: "Status",
    // Core's `jobStatus`, not a local pairing of a word with a tone. It was
    // one — the fifth such table on this branch — and its comment argued it
    // could not use `phaseKind`, which is true and beside the point: a Job's
    // own verdict is `jobStatus`, the very function the Jobs list, the Jobs
    // column table and this CronJob's spawned-Job rows all read. (#331)
    render: (j) => {
      const { status, health } = jobStatus(j.failed, j.active);
      return <StatusPill status={status} kind={health} />;
    },
  },
  { key: "duration", header: "Duration", render: (j) => j.duration || "—" },
  { key: "age", header: "Age", getSortValue: ageSortValue, render: (j) => <AgeCell created={j.created} age={j.age} /> },
];

/**
 * The Jobs this CronJob has spawned — classic's `CronJobJobs`, fetched live
 * via core's `listJobs` and kept to the ones owned by this CronJob (matching
 * classic's own client-side `j.owner === ownerName` filter; core's
 * `listJobs` has no owner parameter of its own). Classic's row click opens
 * the Job; that's the only thing this component does beyond showing rows —
 * no write action — and it renders here as an inert table, same as
 * `WorkloadBody`'s Deploy Revisions.
 */
function RecentJobsSection({
  context,
  namespace,
  ownerName,
}: {
  context: string;
  namespace: string;
  ownerName: string;
}) {
  const state = useSectionList<JobSummary[]>(true, [context, namespace, ownerName], async () => {
    const out = await listJobs(context, namespace);
    return out.error
      ? { error: out.error }
      : { data: (out.jobs ?? []).filter((j) => j.owner === ownerName) };
  });

  if (state.status === "loading") {
    return (
      <Section title="Recent Jobs">
        <LoadingState label="Loading jobs" />
      </Section>
    );
  }
  // The block STAYS on a refusal, with the reason in it. It used to `return
  // null`, so "this CronJob has never run" and "srelens was refused to list
  // jobs" drew the identical screen — see {@link useSectionList}.
  if (state.status === "error") {
    return (
      <Section title="Recent Jobs">
        <SectionFailure error={state.error} />
      </Section>
    );
  }

  return (
    <Section title="Recent Jobs">
      <Table
        columns={RECENT_JOB_COLUMNS}
        data={state.data ?? []}
        getRowKey={(j) => j.name}
        emptyText="No jobs yet"
      />
    </Section>
  );
}

/**
 * A CronJob's Details pane: Schedule, then its recent Jobs (classic's
 * `CronJobBody`). `relatedPodSelector` has no case for "CronJob", so
 * `GenericBody` fetches no related pods for one; its unheaded identity block
 * and its Conditions, Labels and Annotations blocks still wrap this body.
 *
 * Both blocks are returned as siblings, never wrapped: `.section + .section`
 * is what draws the hairline between two blocks, so an element around either
 * of them would take the rule out on both sides of it.
 */
export function CronJobDetailsBody({ object, context }: { object: K8sObject; context: string }) {
  const namespace = str(object.metadata?.namespace);
  const name = str(object.metadata?.name);

  return (
    <>
      <ScheduleSection object={object} />
      {context && namespace && name && (
        <RecentJobsSection context={context} namespace={namespace} ownerName={name} />
      )}
    </>
  );
}
