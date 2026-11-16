import { useEffect, useState } from "react";
import {
  absoluteTimestamp,
  certificateHealth,
  certificateRows,
  decodeBase64,
  decodedByteLength,
  dockerRegistries,
  formatBytes,
  getSecret,
  plural,
  str,
  type CertificateRow,
  type DockerRegistryRow,
  type K8sObject,
} from "@srelens/core";
import { Button, CopyButton, EmptyState, KV, LoadingState, StatusPill, Table, type Column } from "@srelens/ui-kit";
import { FailureAlert } from "../../lib/errorCopy";
import { Section } from "./Section";
import { AnnotationLines, StringList } from "./sections";

/**
 * What this pane knows about a Secret's VALUES — which is a different question
 * from what it knows about the Secret.
 *
 * `k8s.getObject` answers the second: it succeeds, and `redact_secret_data`
 * blanks every value to `""` on the way out while keeping the KEYS. So the key
 * names, the count and the type are facts even when the values are not. The
 * values come from `k8s.getSecret`, a distinct `SENSITIVE_READ` capability with
 * its own consent gate and its own timeout, and it can refuse on its own: a
 * denied consent, a 403 scoped to `secrets`, a timeout.
 *
 * When it does, `data` here is `getObject`'s blanked map — and every fact
 * derived from it is a LIE told confidently. `decodeBase64("")` is `""`, so a
 * key the reader may not read renders identically to a key whose value is
 * empty; `certificateCount("")` is 0, so a TLS Secret that has both halves
 * reads "Missing tls.crt" and "Missing"; `dockerRegistries` finds nothing, so a
 * working pull secret reads "0 registries" and "No valid registry credentials
 * found". That is why this is a STATE and not a bare map: every surface below
 * takes it and WITHHOLDS the assertions it cannot make, rather than deriving
 * them from blanks.
 *
 * `loading` is gated with `error` for the same reason and not as a courtesy:
 * the peek renders this body the moment a row is clicked, with the fetch still
 * in flight, so "not yet" and "not allowed" are the same claim about what the
 * pane knows.
 */
export interface SecretDataState {
  status: "loading" | "ready" | "error";
  /** What to render keys from — the fetched values once `ready`, `getObject`'s
   *  blanked (but correctly KEYED) map otherwise. */
  data: Record<string, string>;
  /** The annotations as the cluster has them once `ready`; `getObject`'s
   *  blanked (but correctly KEYED) map otherwise. */
  annotations: Record<string, string>;
  /** The refusal, raw, exactly as the backend gave it — worded by `describeError`
   *  at the surface, per this package's error rule. */
  error?: string;
  /** Ask for the values again, from `loading`. */
  retry?: () => void;
}

/** Are this Secret's values on hand? The one question every surface below asks. */
function hasValues(state: SecretDataState): boolean {
  return state.status === "ready";
}

/** Why not, in a few words the reader can act on. Kept apart from the
 *  `FailureAlert` above the blocks, which carries the cluster's own sentence:
 *  this is the label beside a withheld control, and it has room for nothing
 *  more. */
function withheldReason(state: SecretDataState): string {
  return state.status === "loading" ? "Reading values…" : "Values not available";
}

/**
 * A private key's format, read straight off its PEM header — classic's
 * `privateKeyType`. Unlike `publicKeyAlgorithm` (see `TlsSection` below),
 * this needs no certificate-parsing dependency: it is a plain regex over the
 * PEM text, so it is ported here directly rather than left for the R-7
 * dependency decision.
 */
function privateKeyType(pem: string): string {
  if (/BEGIN RSA PRIVATE KEY/.test(pem)) return "RSA (PKCS#1)";
  if (/BEGIN EC PRIVATE KEY/.test(pem)) return "EC (SEC1)";
  if (/BEGIN ENCRYPTED PRIVATE KEY/.test(pem)) return "Encrypted PKCS#8";
  if (/BEGIN PRIVATE KEY/.test(pem)) return "PKCS#8";
  return pem ? "Unrecognized format" : "Missing";
}

function certificateCount(pem: string): number {
  return pem.match(/-----BEGIN CERTIFICATE-----/g)?.length ?? 0;
}


const CERTIFICATE_COLUMNS: Column<CertificateRow>[] = [
  { key: "role", header: "Certificate", render: (row) => row.role },
  { key: "subject", header: "Subject", render: (row) => <span className="font-mono">{row.subject}</span> },
  {
    key: "status",
    header: "Status",
    render: (row) => <StatusPill status={row.status} kind={certificateHealth(row.status)} />,
  },
  { key: "size", header: "Size", render: (row) => row.size },
];

/**
 * One Secret entry — key and value, the value shown, decoded, as soon as
 * `k8s.getSecret` has answered (#1085). It used to sit behind a Reveal click
 * per key; the read that fetches it is the audited one either way, so the mask
 * hid nothing from anything but the reader who had just opened the Secret.
 *
 * Hide is the reader's own, per key, for a screen someone else can see — and a
 * hidden value is OUT OF THE DOCUMENT, not merely out of sight: the
 * placeholder "••••••••" is the only value in the tree, and no `title`,
 * `aria-label` or `data-*` carries the value. Copy is offered only over a
 * value that is on screen.
 *
 * The toggle is DISABLED, with the reason beside it, when the values are not on
 * hand ({@link SecretDataState}). Left enabled it decodes `""` and reveals an
 * empty `<pre>` — so a key the reader is not allowed to read looks exactly like
 * a key whose value is empty, which is the pane inventing a fact. Disabled
 * rather than removed: the affordance is what says the value EXISTS and is
 * merely out of reach, and the label beside it says why.
 */
function SecretEntry({ name, value, state }: { name: string; value: string; state: SecretDataState }) {
  const [hidden, setHidden] = useState(false);
  const available = hasValues(state);
  const shown = available && !hidden;
  return (
    <div className="flex flex-col gap-1">
      <div className="flex items-center justify-between gap-2">
        <span className="font-mono text-[0.75rem] font-medium">{name}</span>
        <div className="flex items-center gap-2">
          {!available && <span className="text-[0.75rem] text-muted">{withheldReason(state)}</span>}
          {shown && <CopyButton iconOnly label={`Copy ${name}`} text={() => decodeBase64(value)} />}
          <Button
            type="button"
            variant="ghost"
            size="xs"
            disabled={!available}
            onClick={() => setHidden((h) => !h)}
          >
            {shown ? "Hide" : "Reveal"}
          </Button>
        </div>
      </div>
      <pre className="whitespace-pre-wrap break-all font-mono text-[0.75rem] text-ink">
        {shown ? decodeBase64(value) : "••••••••"}
      </pre>
    </div>
  );
}

/**
 * A Secret's `data` keys, each shown with its own Hide — classic's
 * `SecretData`, shared by all three Secret bodies below (TLS and Docker
 * secrets show it after their own summary section; a general Secret shows it
 * as its only section besides "Secret summary").
 *
 * The key names and the count are `getObject`'s, and they survive a `getSecret`
 * refusal intact — so this section is drawn in full either way, and only the
 * per-key value is withheld.
 */
function SecretDataSection({ state }: { state: SecretDataState }) {
  const keys = Object.keys(state.data);
  return (
    // Remembered as `Data`, not as `Data (3 keys)`: the heading counts what
    // is in the object, and a memory keyed on it would be lost the first
    // time someone added a key.
    <Section id="Data" title={`Data (${plural(keys.length, "key")})`} defaultOpen>
      {keys.length === 0 ? (
        <EmptyState title="No data" />
      ) : (
        <div className="flex flex-col gap-4">
          {keys.map((key) => (
            <SecretEntry key={key} name={key} value={str(state.data[key])} state={state} />
          ))}
        </div>
      )}
    </Section>
  );
}

/**
 * TLS material — classic's `TlsSecretBody`, including every fact that comes
 * from actually parsing the certificate: Certificate status, Subject,
 * Issuer, Serial number, Public key, Valid from/until, DNS/IP names, and the
 * per-certificate table. Those all need `X509Certificate` from
 * `@peculiar/x509`; the R-7 ruling that kept that dependency (and this
 * parsing) out of `@srelens/core` has been overruled, and `certificateRows`
 * now lives in `k8sSecret.ts` — see its doc comment for how the bundling
 * property R-7 protected survives the reversal (the parser is still loaded
 * via a dynamic `import()`, not a static one, so it stays out of the main
 * bundle for every consumer, ui-next included).
 *
 * None of these facts come from the private key. The raw `tls.crt`/`tls.key`
 * bytes are shown beneath, in `SecretDataSection`, like any other key.
 */
function TlsSection({ state }: { state: SecretDataState }) {
  const { data } = state;
  // Every fact below this line comes from the VALUES, so none of it may be
  // stated when the values are not on hand — see {@link SecretDataState}. The
  // parse still runs over the blanks (`count` is 0, so it short-circuits to an
  // empty list); it is the RENDER that withholds, in one place rather than
  // eleven.
  const known = hasValues(state);
  const certificate = decodeBase64(str(data["tls.crt"]));
  const privateKey = decodeBase64(str(data["tls.key"]));
  const count = certificateCount(certificate);
  const [certificates, setCertificates] = useState<CertificateRow[] | null>(null);
  useEffect(() => {
    let active = true;
    if (count === 0) {
      setCertificates([]);
    } else {
      certificateRows(certificate)
        .then((rows) => {
          if (active) setCertificates(rows);
        })
        .catch(() => {
          if (active) setCertificates([]);
        });
    }
    return () => {
      active = false;
    };
  }, [certificate, count]);
  const leaf = certificates?.[0];
  return (
    <Section title="TLS material">
      {/* The Secret's own `type`, off the object rather than off its values —
          a fact whether or not `getSecret` answered, so it stays. */}
      <KV k="Type" v="kubernetes.io/tls" />
      {known ? (
        <>
          <KV k="Certificates" v={count > 0 ? plural(count, "certificate") : "Missing tls.crt"} />
          <KV k="Private key" v={privateKeyType(privateKey)} />
          {leaf && (
            <KV k="Certificate status" v={<StatusPill status={leaf.status} kind={certificateHealth(leaf.status)} />} />
          )}
          {leaf?.subject && <KV k="Subject" v={leaf.subject} />}
          {leaf?.issuer && <KV k="Issuer" v={leaf.issuer} />}
          {leaf?.serial && <KV k="Serial number" v={leaf.serial} mono />}
          {leaf?.keyAlgorithm && <KV k="Public key" v={leaf.keyAlgorithm} />}
          {leaf?.validFrom && <KV k="Valid from" v={absoluteTimestamp(leaf.validFrom)} />}
          {leaf?.validUntil && <KV k="Valid until" v={absoluteTimestamp(leaf.validUntil)} />}
          {leaf && leaf.sans.length > 0 && <KV k="DNS / IP names" v={<StringList items={leaf.sans} />} />}
          {data["tls.crt"] && <KV k="Certificate data" v={formatBytes(decodedByteLength(data["tls.crt"]))} />}
          {data["tls.key"] && <KV k="Private key data" v={formatBytes(decodedByteLength(data["tls.key"]))} />}
          {certificates === null && count > 0 && <LoadingState compact label="Reading certificates" />}
          {certificates && certificates.length > 0 && (
            <Table columns={CERTIFICATE_COLUMNS} data={certificates} getRowKey={(row) => row.key} />
          )}
        </>
      ) : state.status === "loading" ? (
        <LoadingState compact label="Reading this Secret's certificate and private key" />
      ) : (
        <p className="text-[0.8125rem] text-muted">
          The certificate and private key could not be read, so nothing about them is shown here.
        </p>
      )}
    </Section>
  );
}

const DOCKER_COLUMNS: Column<DockerRegistryRow>[] = [
  { key: "registry", header: "Registry", render: (row) => <span className="font-mono">{row.registry}</span> },
  { key: "username", header: "Username", render: (row) => row.username },
  { key: "credential", header: "Credential", render: (row) => row.credential },
];

/**
 * Docker registry credentials — classic's `DockerSecretBody`. `credential`
 * (from core's `dockerRegistries`) is a category ("Stored", "Identity
 * token", "Missing"), never the password/token itself.
 */
function DockerSection({ state, type }: { state: SecretDataState; type: string }) {
  const { data } = state;
  // As in `TlsSection`: the registry list is parsed OUT OF the values, so with
  // the values withheld "0 registries" and "No valid registry credentials
  // found" are two sentences about the read, dressed as two facts about the
  // Secret.
  const known = hasValues(state);
  const configKey = type === "kubernetes.io/dockercfg" ? ".dockercfg" : ".dockerconfigjson";
  const registries = dockerRegistries(data, type);
  return (
    <Section title="Docker registries">
      <KV k="Type" v={type} />
      {known ? (
        <>
          <KV k="Registries" v={plural(registries.length, "registry", "registries")} />
          {data[configKey] && <KV k="Config size" v={formatBytes(decodedByteLength(data[configKey]))} />}
          {registries.length > 0 ? (
            <Table columns={DOCKER_COLUMNS} data={registries} getRowKey={(row) => row.registry} />
          ) : (
            <EmptyState title="No valid registry credentials found" />
          )}
        </>
      ) : state.status === "loading" ? (
        <LoadingState compact label="Reading this Secret's registry credentials" />
      ) : (
        <p className="text-[0.8125rem] text-muted">
          The registry credentials could not be read, so no registries are listed here.
        </p>
      )}
    </Section>
  );
}

/**
 * A generic (Opaque or otherwise unrecognized-type) Secret's summary —
 * classic's `GeneralSecretBody`'s "Secret summary" section: its type, how
 * many keys it holds, and whether it is immutable.
 */
function GeneralSection({ object, keyCount }: { object: K8sObject; keyCount: number }) {
  const immutable = object.immutable === true;
  return (
    <Section title="Secret summary">
      <KV k="Type" v={str(object.type) || "Opaque"} />
      <KV k="Keys" v={plural(keyCount, "key")} />
      <KV k="Immutable" v={immutable ? "Yes" : "No"} />
    </Section>
  );
}

/**
 * Fetch a Secret's real (base64) values via the gated `getSecret` —
 * `getObject` redacts Secret data (the keys are present, the values blank),
 * so this is how the detail view reaches the actual values. The fetched values
 * are shown as soon as they resolve: `SecretEntry` draws each one decoded,
 * with its own Hide (#1085).
 *
 * Returns a {@link SecretDataState} rather than a bare map, and reads `r.error`
 * as well as `r.data`. It used to do neither: `.then((r) => { if (r.data)
 * setData(r.data) })` discarded the failure entirely, so a refusal left the
 * blanked fallback in place with nothing to say it was a fallback — and every
 * surface downstream went on to state those blanks as facts about the Secret.
 * A rejection is read too, not only a returned `error`: `getSecret` catches
 * today, and a hook that only handles one of the two is one refactor away from
 * the same silence.
 *
 * `!context` (a static preview, no cluster to ask) settles as `ready` on the
 * redacted map deliberately: nothing was refused there, and the object handed
 * in is all there is. The one thing that must never be `ready` is a read that
 * was attempted and did not answer.
 */
/**
 * One `k8s.getSecret` read for the surfaces that ask in the same breath. The
 * body and the annotations block mount together and each need the read;
 * without this the audit trail would record two for one opened Secret. Shared
 * only within one tick: a Retry, a later visit, or a read that never settled
 * always reads again.
 */
const sameTick = new Map<string, ReturnType<typeof getSecret>>();
function readSecret(context: string, namespace: string, name: string): ReturnType<typeof getSecret> {
  const key = JSON.stringify([context, namespace, name]);
  let read = sameTick.get(key);
  if (!read) {
    read = getSecret(context, namespace, name);
    sameTick.set(key, read);
    queueMicrotask(() => sameTick.delete(key));
  }
  return read;
}

function useSecretData(
  context: string,
  namespace: string,
  name: string,
  redacted: Record<string, string>,
  blankedAnnotations: Record<string, string> = {},
): SecretDataState {
  const [fetched, setFetched] = useState<Awaited<ReturnType<typeof getSecret>> | null>(null);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    setFetched(null);
    if (!context) return;
    let active = true;
    readSecret(context, namespace, name).then(
      (r) => {
        if (active) setFetched(r);
      },
      (e: unknown) => {
        if (active) setFetched({ error: e instanceof Error ? e.message : String(e) });
      },
    );
    return () => {
      active = false;
    };
  }, [context, namespace, name, attempt]);

  const blanked = { data: redacted, annotations: blankedAnnotations };
  if (!context) return { status: "ready", ...blanked };
  if (fetched === null) return { status: "loading", ...blanked };
  if (fetched.error !== undefined) {
    return { status: "error", ...blanked, error: fetched.error, retry: () => setAttempt((n) => n + 1) };
  }
  return {
    status: "ready",
    data: fetched.data ?? redacted,
    annotations: fetched.annotations ?? blankedAnnotations,
  };
}

/**
 * A Secret's annotations, as the cluster has them, open like any other kind's
 * (#1085). `k8s.getObject` blanks every annotation value on a Secret, so the
 * object the screen holds has the KEYS only; the values are `k8s.getSecret`'s,
 * the same read the body makes.
 *
 * Until that read answers, and when it refuses, the block says so and prints
 * no pairs: `key=` with nothing after it would state that the annotation is
 * empty.
 */
export function SecretAnnotationsSection({
  context,
  namespace,
  name,
  blanked,
}: {
  context: string;
  namespace: string;
  name: string;
  /** `getObject`'s annotations: real keys, blanked values. */
  blanked: Record<string, string>;
}) {
  const state = useSecretData(context, namespace, name, {}, blanked);
  if (Object.keys(blanked).length === 0) return null;
  return (
    <Section title="Annotations">
      {hasValues(state) ? (
        <AnnotationLines annotations={state.annotations} />
      ) : (
        <p className="text-[0.75rem] text-muted">{withheldReason(state)}</p>
      )}
    </Section>
  );
}

/**
 * A Secret's Overview pane — classic's `SecretBody`: dispatches on `type` to
 * TLS material, Docker registries, or a general summary, then always shows
 * the Data section beneath. Editing (`ConfigDataEditor`'s Save/Copy) is not
 * wired here — ui-next's Overview panes are read-only, the same call every
 * other body in this table has made for its own write affordance.
 */
export function SecretDetailsBody({ object, context }: { object: K8sObject; context: string }) {
  const meta = object.metadata ?? {};
  const type = str(object.type) || "Opaque";
  const redacted = (object.data ?? {}) as Record<string, string>;
  const state = useSecretData(context, str(meta.namespace), str(meta.name), redacted);
  // An ALERT and not an error state: the object loaded, so the type, the key
  // names and the count below are real and must stay on screen. What failed is
  // one sub-read, and replacing the pane with a card would throw away the facts
  // the reader does have — the same call `FailureAlert`'s own doc comment makes.
  const refusal =
    state.status === "error" ? (
      <FailureAlert title="This Secret's values could not be read" error={state.error} onRetry={state.retry} />
    ) : null;

  if (type === "kubernetes.io/tls") {
    return (
      <>
        {refusal}
        <TlsSection state={state} />
        <SecretDataSection state={state} />
      </>
    );
  }
  if (type === "kubernetes.io/dockerconfigjson" || type === "kubernetes.io/dockercfg") {
    return (
      <>
        {refusal}
        <DockerSection state={state} type={type} />
        <SecretDataSection state={state} />
      </>
    );
  }
  return (
    <>
      {refusal}
      {/* `getObject` blanks the VALUES and keeps the KEYS, so the count is a
          fact on every path — including a refused one. */}
      <GeneralSection object={object} keyCount={Object.keys(state.data).length} />
      <SecretDataSection state={state} />
    </>
  );
}
