/**
 * The one host-owned confirmation (#552), and the pieces that feed it.
 *
 * Its own entry point rather than a re-export from `@srelens/ui-next`: the
 * classic desktop design mounts two of the three MCP surfaces, and a static
 * import of the package root there would drag the whole new-design shell into
 * a chunk that classic downloads (`design.theme.test.ts`). This subpath pulls
 * in the component, the diff renderer it shares with the Edit screen, and the
 * inventory store — and nothing else.
 */
export {
  HostConfirmation,
  boundedPlainText,
  CONFIRM_DISPLAY_MAX_CHARS,
  type ConfirmationApp,
  type ConfirmationSubject,
  type HostConfirmationProps,
} from "./HostConfirmation";
export { RequestConfirmation } from "./RequestConfirmation";
export { appIdentity, useConfirmationApp, type ConfirmationAppRef } from "./confirmationApp";
export {
  asConfirmRequest,
  asConfirmTarget,
  confirmFields,
  confirmSubject,
  type ConfirmCall,
  type ConfirmFieldValues,
} from "./confirmRequest";
