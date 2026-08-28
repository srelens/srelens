/**
 * Join class names, dropping anything falsy.
 *
 * Local rather than `clsx`, which would be the kit's first runtime dependency
 * for eleven lines of logic. (#318)
 */
export function cx(...parts: Array<string | false | null | undefined>): string {
  return parts.filter(Boolean).join(" ");
}
