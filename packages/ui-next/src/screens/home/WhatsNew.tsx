import { useMemo } from "react";
import { appVersion, checkForUpdate, isTauri, loadUpdateChannel } from "@srelens/core";
import { Button } from "@srelens/ui-kit";
import { FailureAlert } from "../../lib/errorCopy";
import { openTab } from "../../lib/tabsStore";
import { useResource } from "../../lib/useResource";

/**
 * "What's new": whether an update is waiting, by the same check the Release
 * notes screen makes, and the way to its notes.
 *
 * Not drawn on the web host, where the server owns the version and
 * `update_check` is not there to ask. No highlights of the running version
 * are shown: the updater returns notes only for a NEWER version, and nothing
 * in the app carries the current one's — so this names the version and links
 * to the notes rather than inventing a summary.
 */
export function WhatsNew() {
  return isTauri() ? <DesktopWhatsNew /> : null;
}

function DesktopWhatsNew() {
  // Read once, as Release notes does: the channel is a setting.
  const channel = useMemo(() => loadUpdateChannel(), []);
  const found = useResource(async () => {
    const update = await checkForUpdate(channel);
    return { update, current: update ? update.currentVersion : await appVersion() };
  }, [channel]);
  const update = found.data?.update ?? null;

  return (
    <section className="home-side-section" aria-labelledby="home-new-title">
      <h2 id="home-new-title" className="home-section-heading">What's new</h2>
      {found.status === "loading" ? (
        <p className="home-note">Checking for updates…</p>
      ) : found.status === "error" ? (
        <>
          <FailureAlert title="Could not check for updates" error={found.error} domain="http" className="home-section-alert" />
          <div className="home-section-foot">
            <Button variant="secondary" size="sm" aria-label="Retry the update check" onClick={found.reload}>Retry</Button>
          </div>
        </>
      ) : update ? (
        <div className="home-side-group">
          <p className="text-[0.8125rem] font-medium">srelens {update.version} is available</p>
          <p className="text-xs text-muted">You are on {update.currentVersion}.</p>
          <Button variant="primary" size="sm" className="mt-2" aria-label={`See what's new in ${update.version}`} onClick={() => openTab("/notes")}>
            See what&rsquo;s new
          </Button>
        </div>
      ) : (
        <div className="home-side-group">
          <p className="text-[0.8125rem]">srelens {found.data?.current} is up to date</p>
          <Button variant="secondary" size="sm" className="mt-2" onClick={() => openTab("/notes")}>Release notes</Button>
        </div>
      )}
    </section>
  );
}
