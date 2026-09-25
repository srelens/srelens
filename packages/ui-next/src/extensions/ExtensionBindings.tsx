import { useContext, useState, type ReactNode } from "react";
import { CAPABILITY_CATALOG, NETWORK_HTTP, networkHosts, renderConfirmTemplate } from "@srelens/core";
import { CodeEditor } from "@srelens/ui-kit";
import { ExtensionControls } from "./ExtensionControls";
import { escapeFormatCharacters, plainText } from "./displayText";
import { HostText, settingReference, settingTitle } from "./networkText";

// The review reads the manifest as parsed JSON, not as the checked `ExtensionManifest` type:
// it is drawn only once the host has accepted it, but nothing here may throw on a shape the
// host would have refused.
type Fields = Record<string, unknown>;
const fields = (value: unknown): Fields =>
  value && typeof value === "object" && !Array.isArray(value) ? (value as Fields) : {};
const items = (value: unknown): unknown[] => (Array.isArray(value) ? value : []);
/** A manifest value as inline text: strings as written, anything else as its JSON. */
const show = (value: unknown): string =>
  plainText(typeof value === "string" ? value : (JSON.stringify(value) ?? String(value)));

const CUSTOM_RESOURCE = "k8s.listCustomResource";
const EVENTS = "k8s.listEvents";
const SECRET_STORE = "extension.secretStore";
/** The arguments a custom-resource reader's row names in their own cells. */
const RESOURCE_FIELDS: Array<[key: string, label: string]> = [
  ["group", "API group"],
  ["version", "Version"],
  ["kind", "Kind"],
  ["plural", "Plural"],
];
const RESOURCE_KEYS = [...RESOURCE_FIELDS.map(([key]) => key), "namespaced", "printerColumns"];

interface Binding {
  name: string;
  title: string;
  target: string;
  /** A custom-resource reader's accepted API versions, most preferred first (#547). */
  versions: unknown[];
  /** Per listed version, each path the app reads mapped to the one read there instead. */
  overrides: Fields;
  arguments: Fields;
  inputs: unknown[];
}

function bindingsOf(manifest: unknown): Binding[] {
  return items(fields(manifest).capabilities).map((entry) => {
    const binding = fields(entry);
    return {
      name: typeof binding.name === "string" ? binding.name : "",
      title: typeof binding.title === "string" ? binding.title : "",
      target: typeof binding.target === "string" ? binding.target : "",
      versions: items(binding.versions),
      overrides: fields(binding.jsonPathOverrides),
      arguments: fields(binding.arguments),
      inputs: items(binding.inputs),
    };
  });
}

/** A reader's version cell: the one it fixes, or each it accepts in the order tried. */
function Versions({ binding }: { binding: Binding }) {
  const args = binding.arguments;
  if (binding.versions.length === 0)
    return <>{"version" in args ? <code>{show(args.version)}</code> : "Not set"}</>;
  return (
    <>
      {binding.versions.map((version, index) => (
        <span key={index}>
          {index > 0 && ", "}
          <code>{show(version)}</code>
        </span>
      ))}{" "}
      (first served)
    </>
  );
}

/** Each path a reader reads elsewhere at one of its versions, as `version: path → path`. */
const overridesOf = (binding: Binding) =>
  Object.entries(binding.overrides).flatMap(([version, paths]) =>
    Object.entries(fields(paths)).map(([from, to]) => [version, from, to] as const),
  );

function Overrides({ binding }: { binding: Binding }) {
  const overrides = overridesOf(binding);
  if (overrides.length === 0) return <>None</>;
  return (
    <ul aria-label={`${plainText(binding.name)} path overrides`}>
      {overrides.map(([version, from, to], position) => (
        <li key={position}>
          At <code>{show(version)}</code>, <code>{show(from)}</code> is read from <code>{show(to)}</code>
        </li>
      ))}
    </ul>
  );
}

/** The dashboards that show a binding's events, and the API groups each filters them on. */
function eventFilters(manifest: unknown, binding: string) {
  return items(fields(fields(manifest).contributions).pages).flatMap((entry) => {
    const page = fields(entry);
    const events = fields(fields(page.dashboard).events);
    return events.capability === binding
      ? [{ page: page.title ?? page.id, apiGroups: items(events.apiGroups) }]
      : [];
  });
}

const label = (binding: Binding) => plainText(binding.title || binding.name);

/** Fixed arguments as `key` value pairs, or "none". */
function Arguments({ values }: { values: Array<[string, unknown]> }) {
  if (values.length === 0) return <>none</>;
  return (
    <>
      {values.map(([key, value], index) => (
        <span key={key}>
          {index > 0 && ", "}
          <code>{plainText(key)}</code> <code>{show(value)}</code>
        </span>
      ))}
    </>
  );
}

const inputs = (binding: Binding) => (binding.inputs.length ? binding.inputs.map(show).join(", ") : "nothing");

/**
 * One row per custom-resource reader: what it lists and the columns it shows. The column
 * names are on the row and their JSON paths one click away, so a manifest with a dozen
 * readers stays a table rather than a wall.
 */
function CustomResourceReaders({ bindings }: { bindings: Binding[] }) {
  const extra = bindings.map((binding) =>
    Object.entries(binding.arguments).filter(([key]) => !RESOURCE_KEYS.includes(key)),
  );
  const anyExtra = extra.some((values) => values.length > 0);
  const anyOverrides = bindings.some((binding) => overridesOf(binding).length > 0);
  return (
    <div className="extension-binding-table">
      <table aria-label="Custom resources read">
        <thead>
          <tr>
            <th scope="col">Reader</th>
            {RESOURCE_FIELDS.map(([key, name]) => (
              <th scope="col" key={key}>
                {name}
              </th>
            ))}
            <th scope="col">Scope</th>
            <th scope="col">Printer columns</th>
            {anyOverrides && <th scope="col">Path overrides</th>}
            {anyExtra && <th scope="col">Other fixed arguments</th>}
          </tr>
        </thead>
        <tbody>
          {bindings.map((binding, index) => {
            const args = binding.arguments;
            const columns = items(args.printerColumns).map(fields);
            const name = plainText(binding.name);
            return (
              <tr key={`${index}:${binding.name}`} aria-label={`Binding ${name}`}>
                <th scope="row">{label(binding)}</th>
                {RESOURCE_FIELDS.map(([key]) => (
                  <td key={key}>
                    {key === "version" ? (
                      <Versions binding={binding} />
                    ) : key in args ? (
                      <code>{show(args[key])}</code>
                    ) : (
                      "Not set"
                    )}
                  </td>
                ))}
                <td>
                  {args.namespaced === true
                    ? "Namespaced"
                    : args.namespaced === false
                      ? "Cluster-wide"
                      : "Not set"}
                </td>
                <td>
                  {columns.length === 0 ? (
                    "None"
                  ) : (
                    <details>
                      <summary>{columns.map((column) => show(column.name ?? "")).join(", ")}</summary>
                      <table aria-label={`${name} printer columns`}>
                        <thead>
                          <tr>
                            <th scope="col">Column</th>
                            <th scope="col">JSON path</th>
                          </tr>
                        </thead>
                        <tbody>
                          {columns.map((column, position) => (
                            <tr key={position}>
                              <td>{show(column.name ?? "")}</td>
                              <td>
                                <code>{show(column.jsonPath ?? "")}</code>
                              </td>
                            </tr>
                          ))}
                        </tbody>
                      </table>
                    </details>
                  )}
                </td>
                {anyOverrides && (
                  <td>
                    <Overrides binding={binding} />
                  </td>
                )}
                {anyExtra && (
                  <td>
                    <Arguments values={extra[index]} />
                  </td>
                )}
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

/** An event reader reads the namespace in view; a dashboard shows only some API groups' events. */
function EventReaders({ bindings, manifest }: { bindings: Binding[]; manifest: unknown }) {
  return (
    <ul className="extension-binding-readers">
      {bindings.map((binding, index) => {
        const filters = eventFilters(manifest, binding.name);
        const fixed = Object.entries(binding.arguments);
        return (
          <li key={`${index}:${binding.name}`} aria-label={`Binding ${plainText(binding.name)}`}>
            <strong>{label(binding)}</strong>: reads the events of the namespace in view (takes{" "}
            {inputs(binding)} from the view).{" "}
            {filters.length === 0
              ? "No dashboard shows them."
              : filters.map((filter, position) => (
                  <span key={position} className="extension-binding-filter">
                    {show(filter.page ?? "A dashboard")} shows only those for API groups{" "}
                    {filter.apiGroups.map((group, at) => (
                      <span key={at}>
                        {at > 0 && ", "}
                        <code>{show(group)}</code>
                      </span>
                    ))}
                    .{" "}
                  </span>
                ))}
            {fixed.length > 0 && (
              <span>
                Fixed arguments: <Arguments values={fixed} />.
              </span>
            )}
          </li>
        );
      })}
    </ul>
  );
}

/** Any other capability: what it fixes and what it takes from the view, generically. */
function OtherReaders({ bindings }: { bindings: Binding[] }) {
  return (
    <ul className="extension-binding-readers">
      {bindings.map((binding, index) => (
        <li key={`${index}:${binding.name}`} aria-label={`Binding ${plainText(binding.name)}`}>
          <strong>{label(binding)}</strong>: fixed arguments{" "}
          <Arguments values={Object.entries(binding.arguments)} />; takes {inputs(binding)} from the view.
        </li>
      ))}
    </ul>
  );
}

/** A request's URL: the setting it is saved in, or the URL as written. */
function RequestUrl({ manifest, url }: { manifest: unknown; url: unknown }) {
  const id = settingReference(url);
  return id ? <>the URL saved in {settingTitle(manifest, id)}</> : <code>{show(url)}</code>;
}

/** Items joined by commas. */
const listed = (items: ReactNode[]) =>
  items.map((item, index) => (
    <span key={index}>
      {index > 0 && ", "}
      {item}
    </span>
  ));

/**
 * `network.http` (#568): where the app may reach, then what each request sends. The
 * hosts are the grant's scope, and a secret header is named by the setting that keeps
 * it; the value is never in the manifest.
 */
function NetworkRequests({ bindings, manifest }: { bindings: Binding[]; manifest: unknown }) {
  const hosts = networkHosts(manifest);
  return (
    <>
      <p className="extension-message">
        May reach, over HTTPS only (plain HTTP to this computer only if you allow it in the app's details):
      </p>
      <ul className="extension-network-hosts" aria-label="Hosts network.http may reach">
        {hosts.map((host, index) => (
          <li key={index}>
            <HostText manifest={manifest} host={host} />
          </li>
        ))}
      </ul>
      <ul className="extension-binding-readers">
        {bindings.map((binding, index) => {
          const args = binding.arguments;
          const query = Object.entries(fields(args.query));
          const headers = Object.entries(fields(args.headers));
          const secrets = Object.entries(fields(args.secretHeaders)).map(([name, entry]) => {
            const header = fields(entry);
            const prefix = typeof header.prefix === "string" && header.prefix ? header.prefix : null;
            return (
              <>
                secret {settingTitle(manifest, String(header.secret ?? ""))} as the <code>{plainText(name)}</code> header
                {/* Quoted, so a trailing space shows. */}
                {prefix !== null && (
                  <>
                    , after <code>{plainText(JSON.stringify(prefix))}</code>
                  </>
                )}
              </>
            );
          });
          return (
            <li key={`${index}:${binding.name}`} aria-label={`Binding ${plainText(binding.name)}`}>
              <strong>{label(binding)}</strong>: GET <RequestUrl manifest={manifest} url={args.url} />
              {typeof args.path === "string" && (
                <>
                  , path <code>{show(args.path)}</code>
                </>
              )}
              {query.length > 0 && (
                <>
                  , query{" "}
                  {listed(query.map(([key, value]) => <code>{`${plainText(key)}=${show(value)}`}</code>))}
                </>
              )}
              {headers.length > 0 && (
                <>
                  ; {headers.length === 1 ? "header" : "headers"}{" "}
                  {listed(headers.map(([key, value]) => <code>{`${plainText(key)}: ${show(value)}`}</code>))}
                </>
              )}
              {secrets.length > 0 && <>; sends {listed(secrets)}</>}.
            </li>
          );
        })}
      </ul>
    </>
  );
}

/**
 * `extension.secretStore` (#543): not bound to anything, so what it grants is
 * which of the app's settings the host keeps as secrets, plus the host's own
 * metadata for the permission (#548) — read from the capability catalog,
 * never from the manifest.
 */
function SecretStore({ manifest }: { manifest: unknown }) {
  const secrets = items(fields(manifest).settings)
    .map(fields)
    .filter((setting) => setting.type === "secret-reference")
    .map((setting) => show(setting.title ?? setting.id));
  const fact = CAPABILITY_CATALOG.find((capability) => capability.id === SECRET_STORE);
  const asks = fact?.confirm ? renderConfirmTemplate(fact.confirm, {}) : null;
  return (
    <p className="extension-message">
      Keeps these secret settings in srelens's encrypted secrets vault: {secrets.length ? secrets.join(", ") : "none"}. The app
      never reads them; the host uses one only where a host capability declares a place for it.
      {fact && (
        <>
          {" "}
          {[fact.sensitive && "Sensitive", `${fact.impact} impact`].filter(Boolean).join(" · ")}
          {asks && <> · asks “{asks}”</>}
        </>
      )}
    </p>
  );
}

/**
 * What each requested permission is bound to: for a custom-resource reader its group,
 * version, kind, plural, scope and printer columns; for an event reader the API groups its
 * dashboards filter on; for anything else, its fixed arguments. Every value is the
 * manifest's own text, drawn as plain text with invisible characters escaped.
 */
export function ExtensionBindings({ manifest, permissions }: { manifest: unknown; permissions: string[] }) {
  const bindings = bindingsOf(manifest);
  const targets = [...new Set([...permissions, ...bindings.map((binding) => binding.target)])];
  return (
    <div className="extension-bindings">
      <p>What each permission is bound to:</p>
      <ul className="extension-binding-list" aria-label="Permission bindings">
        {targets.map((target) => {
          const bound = bindings.filter((binding) => binding.target === target);
          return (
            <li key={target} aria-label={`${plainText(target)} bindings`}>
              <code>{plainText(target)}</code>
              {target === SECRET_STORE ? (
                <SecretStore manifest={manifest} />
              ) : bound.length === 0 ? (
                <p className="extension-message">No binding uses this permission.</p>
              ) : target === CUSTOM_RESOURCE ? (
                <CustomResourceReaders bindings={bound} />
              ) : target === EVENTS ? (
                <EventReaders bindings={bound} manifest={manifest} />
              ) : target === NETWORK_HTTP ? (
                <NetworkRequests bindings={bound} manifest={manifest} />
              ) : (
                <OtherReaders bindings={bound} />
              )}
            </li>
          );
        })}
      </ul>
    </div>
  );
}

/**
 * The full manifest under review, collapsed until asked for. Format characters are written
 * as JSON escapes, as in an installed app's Details, so the text cannot display as
 * something other than what it holds.
 */
export function ReviewManifest({ text }: { text: string }) {
  const { Button } = useContext(ExtensionControls);
  const [open, setOpen] = useState(false);
  return (
    <>
      <Button variant="secondary" aria-expanded={open} onClick={() => setOpen(!open)}>
        {open ? "Hide manifest" : "View manifest"}
      </Button>
      {open && (
        <CodeEditor
          value={escapeFormatCharacters(text)}
          readOnly
          language="none"
          copy
          ariaLabel="Manifest under review"
          minHeight={160}
          maxHeight={360}
        />
      )}
    </>
  );
}
