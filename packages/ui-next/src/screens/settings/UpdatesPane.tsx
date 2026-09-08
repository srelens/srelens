import { useEffect, useRef, useState } from "react";
import {
  appVersion, checkForUpdate, installUpdate, loadUpdateChannel, parseReleaseNotes,
  loadUpdateNotes, openExternal, relaunchApp, saveUpdateChannel, type UpdateChannel, type UpdateMeta,
} from "@srelens/core";
import { Button, Progress, Select } from "@srelens/ui-kit";
import { FailureAlert } from "../../lib/errorCopy";
import { Notes } from "../Notes";

type Phase = "idle" | "checking" | "current" | "available" | "installing" | "ready" | "restarting";

export function UpdatesPane() {
  const [channel, setChannel] = useState<UpdateChannel>(loadUpdateChannel);
  const [version, setVersion] = useState<string | null>(null);
  const [versionError, setVersionError] = useState<unknown>(null);
  const [versionAttempt, setVersionAttempt] = useState(0);
  const [phase, setPhase] = useState<Phase>("idle");
  const [update, setUpdate] = useState<UpdateMeta | null>(null);
  const [percent, setPercent] = useState<number | null>(null);
  const [error, setError] = useState<{ title: string; cause: unknown } | null>(null);
  const [notes, setNotes] = useState<{ text: string; loading: boolean; error: unknown }>({ text: "", loading: false, error: null });
  const [openError, setOpenError] = useState<unknown>(null);
  const [notesAttempt, setNotesAttempt] = useState(0);
  const releaseVersion = update?.version ?? version;
  const embeddedNotes = update?.notes ?? "";
  useEffect(() => {
    if (!releaseVersion) return;
    let current = true;
    setOpenError(null);
    setNotes({ text: "", loading: true, error: null });
    void loadUpdateNotes({ version: releaseVersion, notes: embeddedNotes }, { refresh: notesAttempt > 0 }).then(
      (text) => { if (current) setNotes({ text, loading: false, error: null }); },
      (error) => { if (current) setNotes({ text: "", loading: false, error }); },
    );
    return () => { current = false; };
  }, [releaseVersion, embeddedNotes, notesAttempt]);
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    setVersionError(null);
    void appVersion().then((value) => { if (alive.current) setVersion(value); }).catch((cause) => { if (alive.current) setVersionError(cause); });
    return () => { alive.current = false; };
  }, [versionAttempt]);
  const busy = phase === "checking" || phase === "installing" || phase === "restarting";
  async function check() {
    setPhase("checking"); setError(null); setUpdate(null);
    try {
      const found = await checkForUpdate(channel);
      if (!alive.current) return;
      setUpdate(found); setPhase(found ? "available" : "current");
      if (found) setVersion(found.currentVersion);
    } catch (cause) {
      if (!alive.current) return;
      setError({ title: "Could not check for updates", cause }); setPhase("idle");
    }
  }
  async function install() {
    setPhase("installing"); setPercent(null); setError(null);
    try {
      await installUpdate(channel, (value) => { if (alive.current) setPercent(value); });
      if (alive.current) setPhase("ready");
    } catch (cause) {
      if (!alive.current) return;
      setError({ title: "Could not install the update", cause }); setPhase("available");
    }
  }
  async function restart() {
    setPhase("restarting"); setError(null);
    try { await relaunchApp(); }
    catch (cause) {
      if (alive.current) setError({ title: "Could not restart srelens", cause });
    } finally { if (alive.current) setPhase("ready"); }
  }
  return (
    <div className="min-w-0 text-[0.8125rem]">
      <section className="flex flex-wrap items-center gap-x-6 gap-y-3 border-b border-rule px-4 py-3" aria-label="Update preferences">
        <div className="min-w-0 flex-1">
          <h2 className="font-semibold text-ink">Updates</h2>
        </div>
        <label className="flex items-center gap-2">
          <span className="text-muted">Update channel</span>
          <Select value={channel} disabled={busy || phase === "ready"}
            options={[{ value: "stable", label: "Stable" }, { value: "dev", label: "Dev" }]}
            onValueChange={(value) => {
              const next = value as UpdateChannel;
              saveUpdateChannel(next); setChannel(next); setPhase("idle"); setUpdate(null); setError(null);
            }} />
        </label>
        {phase !== "installing" && phase !== "ready" && phase !== "restarting" && (
          <Button variant="secondary" disabled={busy} onClick={() => void check()}>{phase === "checking" ? "Checking…" : "Check for updates"}</Button>
        )}
      </section>
      {channel === "dev" && <p className="border-b border-rule px-4 py-2 text-[0.75rem] text-muted">Dev follows rolling pre-releases, which may be less stable.</p>}
      {phase === "current" && <p role="status" className="px-4 py-3">srelens is up to date.</p>}
      {error && <div className="px-4 py-3"><FailureAlert tone="sev" title={error.title} error={error.cause} domain="http" /></div>}
      <section aria-label={releaseVersion ? `Version ${releaseVersion}` : "Installed release"}>
        <div className="flex flex-wrap items-center justify-between gap-3 border-b border-rule px-4 py-3">
          <div>
            <h2 className="font-semibold">{releaseVersion ? <>Version <span>{releaseVersion}</span></> : "Installed release"}</h2>
            <p className="mt-0.5 text-[0.75rem] text-muted">{phase === "ready" || phase === "restarting" ? "Installed — restart to finish" : update ? `Available to update · Installed ${version ?? update.currentVersion}` : "Current release"}</p>
          </div>
          {update && !update.external && phase === "available" && <Button variant="primary" onClick={() => void install()}>Download &amp; install</Button>}
          {update && !update.external && (phase === "ready" || phase === "restarting") && <Button variant="primary" disabled={busy} onClick={() => void restart()}>{phase === "restarting" ? "Restarting…" : "Restart srelens"}</Button>}
        </div>
        {update?.external && <p className="px-4 py-3 text-muted">This install is managed by your system package manager. Update it there. On Arch: <code>paru -Syu</code> or <code>yay -Syu</code>.</p>}
        {update && !update.external && update.elevates && phase === "available" && <p className="px-4 py-2 text-muted">Installing this update needs administrator rights. Your system will ask for your password.</p>}
        {phase === "installing" && <div className="border-b border-rule px-4 py-3">
          <p role="status">{percent === null ? "Downloading…" : `Downloading — ${percent}%`}</p>
          {percent !== null && <Progress value={percent} ariaLabel="Downloading update" />}
        </div>}
        <div className="px-4 py-3">
          <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
            <h3 className="font-semibold">Release notes</h3>
            {releaseVersion && <a className="text-accent underline" href={`https://github.com/srelens/srelens/releases/tag/${encodeURIComponent(`srelens-v${releaseVersion}`)}`} target="_blank" rel="noreferrer" onClick={(event) => {
              event.preventDefault();
              setOpenError(null);
              void openExternal(event.currentTarget.href).catch((cause) => {
                if (alive.current) setOpenError(cause);
              });
            }}>View release on GitHub</a>}
          </div>
          {openError !== null && <FailureAlert tone="sev" title="Could not open the release in your browser" error={openError} domain="http" />}
          {versionError && !releaseVersion ? <div className="flex flex-col items-start gap-2">
            <FailureAlert tone="sev" title="Could not load the installed version" error={versionError} />
            <Button variant="secondary" onClick={() => setVersionAttempt((value) => value + 1)}>Retry installed version</Button>
          </div> : !releaseVersion || notes.loading ? <p className="text-muted">Loading release notes…</p> : notes.error !== null ? <div className="flex flex-col items-start gap-2">
            <FailureAlert tone="sev" title="Could not load release notes" error={notes.error} domain="http" />
            <Button variant="secondary" onClick={() => setNotesAttempt((value) => value + 1)}>Retry release notes</Button>
          </div> : notes.text.trim() ? <Notes blocks={parseReleaseNotes(notes.text)} /> : <p className="text-muted">This release has no published notes.</p>}
        </div>
      </section>
    </div>
  );
}
