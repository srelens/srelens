/**
 * Base64/JSON helpers for Secret data, plus TLS certificate parsing.
 *
 * `publicKeyAlgorithm` and `certificateRows` depend on `X509Certificate` from
 * `@peculiar/x509`. An earlier ruling (R-7) kept that dependency out of the
 * service layer and left this parsing in classic
 * (`apps/desktop/src/components/ResourceOverview.tsx`) alone. That ruling has
 * since been overruled: `@peculiar/x509` is now a declared dependency here so
 * both classic and ui-next can share one implementation.
 *
 * The property that survived the reversal: certificate parsing must still
 * stay out of the *main* bundle. `certificateRows` therefore keeps classic's
 * `await import("@peculiar/x509")` (preceded by `await import("reflect-metadata")`,
 * which the library needs for its decorator metadata) inside the function
 * body rather than a static top-level import — only `import type` reaches the
 * module scope, which erases at build time and carries no runtime weight.
 * Whoever calls `certificateRows` still only pays for the parser's bytes when
 * that code path actually runs.
 */
import type { HealthKind } from "./k8sHealth";
import { asRecord, str } from "./k8sRaw";
import { formatBytes } from "./k8sQuantity";
import type { X509Certificate } from "@peculiar/x509";

export function decodeBase64(v: string): string {
  try {
    const binary = atob(v);
    const bytes = Uint8Array.from(binary, (character) => character.charCodeAt(0));
    return new TextDecoder().decode(bytes);
  } catch {
    return v;
  }
}

export interface DockerRegistryRow {
  registry: string;
  username: string;
  credential: string;
}

export function dockerRegistries(data: Record<string, string>, type: string): DockerRegistryRow[] {
  const key = type === "kubernetes.io/dockercfg" ? ".dockercfg" : ".dockerconfigjson";
  try {
    const parsed = JSON.parse(decodeBase64(str(data[key]))) as Record<string, unknown>;
    const auths = type === "kubernetes.io/dockercfg" ? parsed : asRecord(parsed.auths);
    return Object.entries(auths).map(([registry, raw]) => {
      const auth = asRecord(raw);
      const decodedAuth = auth.auth ? decodeBase64(str(auth.auth)) : "";
      // `auth` is conventionally base64 of `username:password`, but some
      // registries store a bare token there with no separator at all. On a
      // colon-less string, `split(":", 1)[0]` returns the whole string — the
      // entire credential — so the split only counts when a colon is
      // actually present; a colon-less `auth` yields no username, not the
      // whole decoded value.
      const colonIndex = decodedAuth.indexOf(":");
      const username = str(auth.username) || (colonIndex >= 0 ? decodedAuth.slice(0, colonIndex) : "");
      return {
        registry,
        username: username || "—",
        credential: auth.identitytoken ? "Identity token" : auth.auth || auth.password ? "Stored" : "Missing",
      };
    });
  } catch {
    return [];
  }
}

/** One certificate in a TLS Secret's chain, as classic's `TlsSecretBody` table
 *  renders it: role in the chain, the facts parsed off it, and its encoded
 *  size. `status`/`subject`/etc. are "" or "Invalid" (never omitted) when the
 *  PEM block itself failed to parse. */
/**
 * The five words {@link certificateRows} can put on a certificate, and the
 * whole of that vocabulary. A union rather than `string` so
 * {@link CERTIFICATE_HEALTH} below is a TOTAL table: a sixth word added here
 * is a compile error at the tone table rather than a silent red pill.
 */
export type CertificateStatus = "Valid" | "Expires soon" | "Expired" | "Not yet valid" | "Invalid";

export interface CertificateRow {
  key: string;
  role: string;
  subject: string;
  issuer: string;
  serial: string;
  validFrom: string;
  validUntil: string;
  status: CertificateStatus;
  keyAlgorithm: string;
  sans: string[];
  size: string;
}

/**
 * The tone each certificate word carries — the pairing, beside the vocabulary
 * that produces it.
 *
 * ui-next held this as `certificateStatusKind`, a three-branch ternary whose
 * `else` swept "Expired", "Not yet valid" and "Invalid" into danger. That is
 * the sixth hand-paired word/tone table found on this branch, and it had the
 * same defect as the other five: it lived in a different package from the
 * words, so it could only be checked by reading both. Enumerated here instead,
 * one entry per word, and total by construction.
 *
 * "Not yet valid" is amber, not red, and that judgement belongs with the
 * vocabulary: a certificate whose `notBefore` is in the future is a rotation
 * staged early, not a broken one — the same call core already makes for a
 * running Job (amber, no dot) versus a failed one. `Expired` and `Invalid`
 * are red because they are the two states where TLS is actually not working.
 */
const CERTIFICATE_HEALTH: Record<CertificateStatus, HealthKind> = {
  Valid: "success",
  "Expires soon": "warning",
  "Not yet valid": "warning",
  Expired: "danger",
  Invalid: "danger",
};

/** The tone for a certificate's status word. */
export function certificateHealth(status: CertificateStatus): HealthKind {
  return CERTIFICATE_HEALTH[status];
}

/** The public key's algorithm and size/curve, e.g. "RSASSA-PKCS1-v1_5
 *  2048-bit" or "ECDSA P-256" — moved byte-for-byte from classic's
 *  `ResourceOverview.tsx` per the R-7 reversal (see this file's header). */
export function publicKeyAlgorithm(certificate: X509Certificate): string {
  const algorithm = certificate.publicKey.algorithm as Algorithm & {
    modulusLength?: number;
    namedCurve?: string;
  };
  if (algorithm.namedCurve) return `${algorithm.name} ${algorithm.namedCurve}`;
  if (algorithm.modulusLength) return `${algorithm.name} ${algorithm.modulusLength}-bit`;
  return algorithm.name;
}

/** The PEM armour a certificate block opens and closes with. */
const PEM_BEGIN = "-----BEGIN CERTIFICATE-----";
const PEM_END = "-----END CERTIFICATE-----";

/**
 * Every PEM certificate block in `pem`, `-----BEGIN` through `-----END`.
 *
 * Deliberately not a regex. `-----BEGIN CERTIFICATE-----[\s\S]*?-----END
 * CERTIFICATE-----/g` made every BEGIN a fresh start position for a lazy body
 * that then rescanned to the end of the string looking for an END: input with
 * many openers and no closer cost 1386ms at 328KB and grew quadratically
 * (js/polynomial-redos, #380). This is a Secret's own `tls.crt` as the cluster
 * hands it over, parsed on the UI thread, so its shape is not ours to trust.
 *
 * Two `indexOf` walks with a cursor that only ever moves forward is linear,
 * and gives byte-identical blocks: the block starts at the earliest BEGIN and
 * ends at the first END after it, exactly as the lazy pattern did — a nested
 * BEGIN is swallowed by the block that opened first, an unclosed BEGIN yields
 * nothing, and a stray END before any BEGIN is not a block.
 */
export function certificateBlocks(pem: string): string[] {
  const blocks: string[] = [];
  let from = 0;
  for (;;) {
    const begin = pem.indexOf(PEM_BEGIN, from);
    if (begin === -1) return blocks;
    const end = pem.indexOf(PEM_END, begin + PEM_BEGIN.length);
    if (end === -1) return blocks;
    blocks.push(pem.slice(begin, end + PEM_END.length));
    from = end + PEM_END.length;
  }
}

/** Parse every PEM certificate block in `pem` into a {@link CertificateRow}.
 *  The first block is the leaf; every block after it is a numbered chain
 *  entry. A block that fails to parse still gets a row — "Invalid" status,
 *  empty facts, but a real size — rather than being dropped, so the table's
 *  certificate count always matches what `pem` actually contained. */
export async function certificateRows(pem: string): Promise<CertificateRow[]> {
  await import("reflect-metadata");
  const { SubjectAlternativeNameExtension, X509Certificate } = await import("@peculiar/x509");
  return certificateBlocks(pem).map((pemCertificate, index) => {
    const fallback: CertificateRow = {
      key: String(index),
      role: index === 0 ? "Leaf" : `Chain ${index}`,
      subject: "Unable to parse certificate",
      issuer: "",
      serial: "",
      validFrom: "",
      validUntil: "",
      status: "Invalid",
      keyAlgorithm: "",
      sans: [],
      size: formatBytes(new TextEncoder().encode(pemCertificate).length),
    };
    try {
      const certificate = new X509Certificate(pemCertificate);
      const now = Date.now();
      const expires = certificate.notAfter.getTime();
      const starts = certificate.notBefore.getTime();
      const status: CertificateStatus = now < starts
        ? "Not yet valid"
        : now > expires
          ? "Expired"
          : expires - now < 30 * 86_400_000
            ? "Expires soon"
            : "Valid";
      const san = certificate.getExtension(SubjectAlternativeNameExtension);
      return {
        ...fallback,
        subject: certificate.subject,
        issuer: certificate.issuer,
        serial: certificate.serialNumber,
        validFrom: certificate.notBefore.toISOString(),
        validUntil: certificate.notAfter.toISOString(),
        status,
        keyAlgorithm: publicKeyAlgorithm(certificate),
        sans: san?.names.items.map((name) => name.value) ?? [],
      };
    } catch {
      return fallback;
    }
  });
}
