import { on } from "../transport/transport";
import { notify } from "./notify";

/**
 * Something the desktop host has to tell a person after the page that would
 * have heard it is gone (#735).
 *
 * The case that needs it: a helm operation whose window closed or reloaded
 * while it ran. The host never kills one — killing helm partway through a
 * release is what leaves it stuck — so the operation finishes with no page
 * listening, and how it ended would otherwise be known only to the app log.
 * The host broadcasts the outcome to every open window instead, the reloaded
 * one included, and each shows it as a toast.
 */
export interface HostNotice {
  /** `error` for something that failed; anything else is information. */
  level: "info" | "error";
  title: string;
  detail?: string;
}

/** The event the host broadcasts a {@link HostNotice} on. */
const HOST_NOTICE = "host-notice";

/** A notice as a `notify` toast — the classic design's surface. */
function toast(notice: HostNotice): void {
  if (notice.level === "error") notify.error(notice.title, notice.detail);
  else notify.info(notice.title, notice.detail);
}

/**
 * Hand each notice the host broadcasts to `show`, for as long as the page
 * lives: a `notify` toast unless the caller draws notices itself — the new
 * design does, so a notice stays up until it is dismissed. Returns the subscription's
 * release.
 */
export function listenForHostNotices(show: (notice: HostNotice) => void = toast): () => void {
  return on(HOST_NOTICE, (payload) => {
    const notice = payload as Partial<HostNotice> | null;
    if (typeof notice?.title !== "string") return;
    show({
      level: notice.level === "error" ? "error" : "info",
      title: notice.title,
      detail: notice.detail,
    });
  });
}
