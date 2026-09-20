/**
 * Writes a character as JSON escapes, one `\uXXXX` per UTF-16 unit, so a code point above
 * U+FFFF (a tag character, say) becomes its surrogate pair and the text still reads back as
 * the same value.
 */
const escapeUnits = (character: string) =>
  character
    .split("")
    .map((unit) => `\\u${unit.charCodeAt(0).toString(16).padStart(4, "0")}`)
    .join("");

/**
 * Manifest JSON for display, with every format character (category Cf: bidirectional
 * overrides and isolates, zero-width spaces and joiners, tag characters) written as a JSON
 * escape rather than drawn. `JSON.stringify` escapes control characters but not these, and
 * drawn they can reorder or hide the text around them. The result is still the same JSON.
 */
export function escapeFormatCharacters(json: string): string {
  return json.replace(/\p{Cf}/gu, escapeUnits);
}

/**
 * One manifest value drawn inline in host UI, such as a printer column's name or JSON path.
 * React renders it as text, never markup; this also writes format, control and line or
 * paragraph separator characters as `\uXXXX`, so the value cannot reorder, hide or break
 * the host's text around it, whatever the host's checks let through.
 */
export function plainText(value: string): string {
  return value.replace(/[\p{Cc}\p{Cf}\p{Zl}\p{Zp}]/gu, escapeUnits);
}
