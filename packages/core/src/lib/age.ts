// Sort keys for srelens compact age strings (#236). The backend renders ages
// as a single value+unit pair ("45s", "3m", "2h", "300d", "1y" — see
// `format_age` in crates/kube), which the table's numeric collation orders
// wrong across units: "1y" sorted before "300d" because 1 < 300. Parsing the
// string back to seconds gives the comparator a real duration.

const UNIT_SECONDS: Record<string, number> = {
  s: 1,
  m: 60,
  h: 3_600,
  d: 86_400,
  y: 86_400 * 365,
};

/**
 * The duration (seconds) a compact age string denotes, for sorting. Unset
 * ("-") or unrecognized ages return -1 so they group together below every
 * real age instead of interleaving arbitrarily.
 */
export function ageSeconds(age: string): number {
  const match = /^(\d+)([smhdy])$/.exec(age.trim());
  if (!match) return -1;
  return Number(match[1]) * UNIT_SECONDS[match[2]];
}

/**
 * Drop-in `getSortValue` for any summary row carrying a compact `age` —
 * one shared function rather than an arrow per column definition.
 */
export function ageSortValue(row: { created?: string | null; age?: string }): number {
  // `created` is the real thing, so it sorts on a real duration — no parsing,
  // no unit table, and correct between two rows the compact string renders
  // identically (two Secrets 3m10s and 3m50s old are both "3m"). Rows whose
  // kind does not carry a timestamp yet fall back to reading the string, which
  // is what #236 added this function for.
  if (row.created) {
    const then = new Date(row.created).getTime();
    if (!Number.isNaN(then)) return Math.max(0, Math.floor((Date.now() - then) / 1000));
  }
  return ageSeconds(row.age ?? "");
}
