import { useContext, useState } from "react";
import { CodeEditor } from "@srelens/ui-kit";
import { ExtensionControls } from "./ExtensionControls";
import { escapeFormatCharacters, plainText } from "./displayText";

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
      arguments: fields(binding.arguments),
      inputs: items(binding.inputs),
    };
  });
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
                  <td key={key}>{key in args ? <code>{show(args[key])}</code> : "Not set"}</td>
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
              {bound.length === 0 ? (
                <p className="extension-message">No binding uses this permission.</p>
              ) : target === CUSTOM_RESOURCE ? (
                <CustomResourceReaders bindings={bound} />
              ) : target === EVENTS ? (
                <EventReaders bindings={bound} manifest={manifest} />
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
