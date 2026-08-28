import { useMemo, useState } from "react";
import { Button, EmptyState, LoadingState, Progress, Screen } from "@srelens/ui-kit";
import {
  appVersion,
  checkForUpdate,
  installUpdate,
  isTauri,
  loadUpdateChannel,
  parseReleaseNotes,
  type UpdateMeta,
} from "@srelens/core";
import { FailureAlert, FailureState } from "../lib/errorCopy";
import { useResource } from "../lib/useResource";
import { Notes } from "./Notes";

/**
 * Where the install has got to. `percent` is null until a total size arrives.
 *
 * A failure carries the caught value ITSELF, not a message read off it. The
 * screen used to keep `cause instanceof Error ? cause.message : String(cause)`
 * and print that at the reader; `describeError` classifies an object and keeps
 * the original for the disclosure, and neither is possible once the value has
 * been flattened to a string on the way in.
 */
type Install =
  | { phase: "idle" }
  | { phase: "installing"; percent: number | null }
  | { phase: "done" }
  | { phase: "failed"; cause: unknown };

/**
 * What is in the update that is waiting, and the button that takes it.
 *
 * The check runs on mount rather than behind a "check now" button: the screen
 * exists to answer one question, and making the user ask it twice — once by
 * opening the screen, once by clicking — buys nothing. `useResource` owns the
 * four states so a failed check offers a retry instead of an empty page.
 *
 * The installed version comes from the update's own `currentVersion` whenever
 * there is an update, and only falls back to `appVersion()` when there is
 * none: the updater already told us what it compared against, and a second
 * round trip to ask the same question could answer differently.
 *
 * `route` is part of every screen's shape; this one has a single route and
 * nothing to read out of it, so the prop is accepted and unused.
 */
export function ReleaseNotes(_props: { route: string }) {
  // The server owns updates in web mode, so there is nothing here to check —
  // and `update_check` is a desktop capability that would fail if we asked.
  const web = !isTauri();
  // Read once: the channel is a setting, and re-reading it every render would
  // put a new value into the loader's dependencies on every render.
  const channel = useMemo(() => loadUpdateChannel(), []);
  const [install, setInstall] = useState<Install>({ phase: "idle" });

  const found = useResource(
    async () => {
      if (web) return { update: null as UpdateMeta | null, current: "" };
      const update = await checkForUpdate(channel);
      return { update, current: update ? update.currentVersion : await appVersion() };
    },
    [web, channel],
    (v) => v.update === null,
  );

  // Only a ready load carries data, so this is null in every other state.
  const update = web ? null : (found.data?.update ?? null);

  const start = async () => {
    setInstall({ phase: "installing", percent: null });
    try {
      await installUpdate(channel, (percent) => setInstall({ phase: "installing", percent }));
      setInstall({ phase: "done" });
    } catch (cause) {
      setInstall({ phase: "failed", cause });
    }
  };

  return (
    <Screen
      title="Release notes"
      eyebrow={update ? `Update to ${update.version}` : undefined}
      description={update ? `You are on ${update.currentVersion}.` : undefined}
    >
      {web ? (
        <EmptyState
          title="Updates are managed by the server"
          hint="srelens changes version when whoever runs it deploys a new one."
        />
      ) : found.status === "loading" ? (
        <LoadingState label="Checking for updates" />
      ) : found.status === "error" ? (
        // Through `lib/errorCopy` rather than the kit's raw `ErrorState`: the
        // updater's own refusals reach the reader classified, with the original
        // folded away behind a disclosure. The screen's title stays, because
        // that is the half `describeError` cannot know.
        <FailureState
          title="Could not check for updates"
          error={found.error}
          onRetry={found.reload}
        />
      ) : !update ? (
        <EmptyState
          title="srelens is up to date"
          hint={found.data ? `Version ${found.data.current}` : undefined}
        />
      ) : (
        <div className="flex flex-col gap-3">
          <Notes blocks={parseReleaseNotes(update.notes)} />
          {update.external ? (
            // Nothing to press: the installed copy belongs to a package
            // manager the updater cannot drive.
            <p className="text-[0.8125rem] text-muted">
              Installed by your package manager — update it there
            </p>
          ) : (
            <div className="flex flex-col items-start gap-2">
              {update.elevates && (
                <p className="text-[0.8125rem] text-muted">srelens will ask for your password</p>
              )}
              <Button
                type="button"
                variant="primary"
                disabled={install.phase === "installing" || install.phase === "done"}
                onClick={() => void start()}
              >
                Install
              </Button>
            </div>
          )}
          {install.phase === "installing" && (
            <div className="flex max-w-[320px] flex-col gap-1.5">
              <p className="text-[0.8125rem] text-muted">
                {install.percent === null ? "Installing…" : `Installing… ${install.percent}%`}
              </p>
              {install.percent !== null && (
                <Progress value={install.percent} ariaLabel="Installing update" />
              )}
            </div>
          )}
          {install.phase === "done" && (
            <p className="text-[0.8125rem]">Restart srelens to finish</p>
          )}
          {install.phase === "failed" && (
            // `sev`, so the kit gives it `role="alert"` — the assertive live
            // region the hand-written paragraph had, kept, because this arrives
            // while the reader is looking at the notes. The notes stay on screen
            // under it: a failed install has not taken them away.
            <FailureAlert tone="sev" title="Could not install the update" error={install.cause} />
          )}
        </div>
      )}
    </Screen>
  );
}
