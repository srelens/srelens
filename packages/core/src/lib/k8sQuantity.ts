export function decodedByteLength(v: string): number {
  try {
    return atob(v).length;
  } catch {
    return new TextEncoder().encode(v).length;
  }
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
}

/** Parse a Kubernetes quantity (e.g. "500m", "2Gi", "4") to a base-unit number. */
export function parseQuantity(q: string): number | null {
  const m = /^([0-9.]+)\s*([a-zA-Z]*)$/.exec((q ?? "").trim());
  if (!m) return null;
  const n = parseFloat(m[1]);
  if (Number.isNaN(n)) return null;
  const unit = m[2];
  const binary: Record<string, number> = { Ki: 2 ** 10, Mi: 2 ** 20, Gi: 2 ** 30, Ti: 2 ** 40, Pi: 2 ** 50, Ei: 2 ** 60 };
  const decimal: Record<string, number> = { k: 1e3, M: 1e6, G: 1e9, T: 1e12, P: 1e15, E: 1e18 };
  if (unit === "") return n;
  if (unit === "m") return n / 1000;
  if (binary[unit]) return n * binary[unit];
  if (decimal[unit]) return n * decimal[unit];
  return n; // unknown unit — same on both sides, so the ratio still holds
}

export function usagePercent(used: string, hard: string): number | null {
  const u = parseQuantity(used);
  const h = parseQuantity(hard);
  if (u == null || h == null || h === 0) return null;
  return Math.round((u / h) * 100);
}
