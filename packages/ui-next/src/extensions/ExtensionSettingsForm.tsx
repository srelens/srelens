import { useContext, useId, useState, type ReactNode } from "react";
import {
  getLiveKubeconfigFiles,
  listContexts,
  listNamespaces,
  type ExtensionSetting,
  type InstalledExtension,
} from "@srelens/core";
import { ExtensionControls } from "./ExtensionControls";
import { ErrorNotice } from "./ExtensionResults";
import { plainText } from "./displayText";
import { extensionLabel } from "./inventoryStore";
import { useResource } from "../lib/useResource";

/** What a field holds while it is edited: text as typed, a box, or the options ticked. */
type Draft = Record<string, string | boolean | string[] | undefined>;

const isSecret = (setting: ExtensionSetting) => setting.type === "secret-reference";

/** The form's starting point: what is saved, never the default, which is only shown. */
function initial(declared: ExtensionSetting[], saved: Record<string, unknown>): Draft {
  const draft: Draft = {};
  for (const setting of declared) {
    const value = saved[setting.id];
    if (isSecret(setting) || value === undefined || value === null) continue;
    if (setting.type === "boolean") draft[setting.id] = value === true;
    else if (setting.type === "multi-select") draft[setting.id] = Array.isArray(value) ? value.map(String) : [];
    else draft[setting.id] = String(value);
  }
  return draft;
}

/** How a default reads beside its field. */
function describeDefault(setting: ExtensionSetting): string | undefined {
  const value = setting.default;
  if (value === undefined || value === null) return undefined;
  const label = (option: unknown) =>
    plainText(setting.options?.find((o) => o.value === option)?.label ?? String(option));
  if (setting.type === "boolean") return `Default: ${value ? "on" : "off"}`;
  if (setting.type === "multi-select") return `Default: ${Array.isArray(value) && value.length ? value.map(label).join(", ") : "none"}`;
  if (setting.type === "select") return `Default: ${label(value)}`;
  return `Default: ${plainText(String(value))}`;
}

/**
 * What the form sends, and what it can already tell is wrong. The host checks
 * everything again and has the last word; these are only the checks that save
 * a round trip.
 */
function collect(declared: ExtensionSetting[], draft: Draft) {
  const values: Record<string, unknown> = {};
  const problems: Record<string, string> = {};
  for (const setting of declared) {
    if (isSecret(setting)) continue;
    const value = draft[setting.id];
    const empty = value === undefined || value === "" || (Array.isArray(value) && value.length === 0);
    if (empty) {
      if (setting.required) problems[setting.id] = "Required";
      continue;
    }
    if (setting.type === "number") {
      const number = Number(value);
      if (typeof value !== "string" || value.trim() === "" || !Number.isFinite(number)) problems[setting.id] = "Enter a number";
      else if (setting.integer && !Number.isInteger(number)) problems[setting.id] = "Enter a whole number";
      else if (setting.minimum !== undefined && number < setting.minimum) problems[setting.id] = `Enter at least ${setting.minimum}`;
      else if (setting.maximum !== undefined && number > setting.maximum) problems[setting.id] = `Enter at most ${setting.maximum}`;
      else values[setting.id] = number;
      continue;
    }
    values[setting.id] = value;
  }
  return { values, problems };
}

/**
 * The host's refusal, split into the lines that name one setting
 * (`settings.<id>: why (CODE)`) and the rest, which say what else refused the
 * values — a binding that interpolates one, say.
 */
function splitRefusal(message: string) {
  const fields: Record<string, string> = {};
  const rest: string[] = [];
  for (const line of message.split("\n")) {
    const match = /(?:^|\s)settings\.([A-Za-z0-9-]+): (.*?)(?: \(EXTENSION_[A-Z_]+\))?$/.exec(line);
    if (match) fields[match[1]] = match[2];
    else if (line.trim()) rest.push(line.trim());
  }
  return { fields, rest: rest.join("\n") };
}

/**
 * An installed app's settings, drawn from the settings its manifest declares
 * (#542). Everything the app wrote here — titles, help, option labels — is its
 * text and is drawn as plain text. A secret has no input: its value is kept by
 * the host's secret store, never in these settings.
 */
export function ExtensionSettingsForm({
  plugin,
  onSave,
  onClose,
}: {
  plugin: InstalledExtension;
  /** Saves through the host, which checks every value; rejects with the host's reason. */
  onSave(settings: Record<string, unknown>): Promise<void>;
  onClose(): void;
}) {
  const { Button } = useContext(ExtensionControls);
  const declared = plugin.manifest.settings ?? [];
  const name = extensionLabel(plugin);
  const [draft, setDraft] = useState<Draft>(() => initial(declared, plugin.settings));
  const [problems, setProblems] = useState<Record<string, string>>({});
  const [failure, setFailure] = useState("");
  const [saved, setSaved] = useState(false);
  const [saving, setSaving] = useState(false);
  const set = (id: string, value: Draft[string]) => {
    setDraft((current) => ({ ...current, [id]: value }));
    setSaved(false);
  };

  async function save() {
    const { values, problems: found } = collect(declared, draft);
    setProblems(found);
    setFailure("");
    setSaved(false);
    if (Object.keys(found).length) return;
    setSaving(true);
    try {
      await onSave(values);
      setSaved(true);
    } catch (e) {
      const { fields, rest } = splitRefusal(e instanceof Error ? e.message : String(e));
      setProblems(fields);
      setFailure(rest || (Object.keys(fields).length ? "" : "The settings were not saved."));
    } finally {
      setSaving(false);
    }
  }

  return (
    <form
      className="extension-settings"
      aria-label={`${name} settings`}
      noValidate
      onSubmit={(event) => {
        event.preventDefault();
        void save();
      }}
    >
      <strong>Settings</strong>
      {declared.length === 0 ? (
        <p className="extension-message">This app declares no settings.</p>
      ) : (
        <>
          <p className="extension-message">The host checks every value when you save, whatever this form allows.</p>
          {failure && (
            <p role="alert" className="extension-error">
              {failure}
            </p>
          )}
          {declared.map((setting) => (
            <SettingField
              key={setting.id}
              setting={setting}
              value={draft[setting.id]}
              saved={plugin.settings[setting.id]}
              problem={problems[setting.id]}
              onChange={(value) => set(setting.id, value)}
            />
          ))}
        </>
      )}
      <div className="extension-toolbar extension-settings-actions">
        {declared.length > 0 && (
          <Button type="submit" disabled={saving}>
            Save settings
          </Button>
        )}
        <Button type="button" variant="secondary" onClick={onClose}>
          Close
        </Button>
        {saved && (
          <span role="status" className="extension-settings-saved">
            Settings saved.
          </span>
        )}
      </div>
    </form>
  );
}

function SettingField({
  setting,
  value,
  saved,
  problem,
  onChange,
}: {
  setting: ExtensionSetting;
  value: Draft[string];
  saved: unknown;
  problem?: string;
  onChange(value: Draft[string]): void;
}) {
  const id = useId();
  const title = plainText(setting.title);
  const help = setting.description ? plainText(setting.description) : undefined;
  const fallback = describeDefault(setting);
  const described = [help && `${id}-help`, fallback && `${id}-default`, problem && `${id}-problem`]
    .filter(Boolean)
    .join(" ") || undefined;
  const notes = (
    <>
      {help && (
        <p id={`${id}-help`} className="extension-setting-note">
          {help}
        </p>
      )}
      {fallback && (
        <p id={`${id}-default`} className="extension-setting-note">
          {fallback}
        </p>
      )}
      {problem && (
        <p id={`${id}-problem`} className="extension-setting-problem">
          {plainText(problem)}
        </p>
      )}
    </>
  );
  const invalid = problem ? true : undefined;
  const text = typeof value === "string" ? value : "";

  switch (setting.type) {
    case "string":
    case "url":
    case "number":
      return (
        <div className="extension-setting">
          <label htmlFor={id}>
            {title}
            {setting.required && <span className="extension-setting-required"> (required)</span>}
          </label>
          <input
            id={id}
            type={setting.type === "number" ? "number" : setting.type === "url" ? "url" : "text"}
            inputMode={setting.type === "number" ? (setting.integer ? "numeric" : "decimal") : undefined}
            value={text}
            maxLength={setting.type === "string" ? (setting.maxLength ?? 1024) : undefined}
            min={setting.minimum}
            max={setting.maximum}
            step={setting.type === "number" ? (setting.integer ? 1 : "any") : undefined}
            spellCheck={false}
            aria-required={setting.required || undefined}
            aria-invalid={invalid}
            aria-describedby={described}
            onChange={(event) => onChange(event.target.value)}
          />
          {notes}
        </div>
      );
    case "boolean":
      return (
        <div className="extension-setting">
          <label className="extension-setting-check">
            <input
              type="checkbox"
              checked={typeof value === "boolean" ? value : setting.default === true}
              aria-invalid={invalid}
              aria-describedby={described}
              onChange={(event) => onChange(event.target.checked)}
            />
            {title}
          </label>
          {notes}
        </div>
      );
    case "multi-select": {
      const chosen = Array.isArray(value) ? value : [];
      return (
        <fieldset className="extension-setting" aria-invalid={invalid} aria-describedby={described}>
          <legend>
            {title}
            {setting.required && <span className="extension-setting-required"> (required)</span>}
          </legend>
          {(setting.options ?? []).map((option) => (
            <label key={option.value} className="extension-setting-check">
              <input
                type="checkbox"
                checked={chosen.includes(option.value)}
                onChange={(event) =>
                  onChange(
                    event.target.checked
                      ? [...chosen, option.value]
                      : chosen.filter((each) => each !== option.value),
                  )
                }
              />
              {plainText(option.label)}
            </label>
          ))}
          {notes}
        </fieldset>
      );
    }
    case "select":
      return (
        <PickedField setting={setting} title={title} notes={notes}>
          <Choice
            label={title}
            value={text}
            none={setting.default === undefined ? "Not set" : "Use the default"}
            options={(setting.options ?? []).map((option) => ({ value: option.value, label: plainText(option.label) }))}
            onChange={onChange}
          />
        </PickedField>
      );
    case "cluster-selector":
      return (
        <PickedField setting={setting} title={title} notes={notes}>
          <ClusterChoice label={title} value={text} onChange={onChange} />
        </PickedField>
      );
    case "namespace-selector":
      return (
        <fieldset className="extension-setting" aria-invalid={invalid} aria-describedby={described}>
          <legend>
            {title}
            {setting.required && <span className="extension-setting-required"> (required)</span>}
          </legend>
          <NamespaceChoice label={title} value={text} onChange={onChange} />
          {notes}
        </fieldset>
      );
    case "secret-reference": {
      const isSet = typeof saved === "object" && saved !== null && "secretRef" in saved;
      return (
        <fieldset className="extension-setting" aria-describedby={described}>
          <legend>{title}</legend>
          <p className="extension-setting-note">
            <strong>{isSet ? "Set" : "Not set"}</strong>
            {" · "}
            A secret is kept in the system keychain, never in these settings or in exported settings. This version
            cannot store one yet, so the app cannot use this setting.
          </p>
          {notes}
        </fieldset>
      );
    }
  }
}

/** A field drawn with a picker, which is labelled by its title rather than a `<label>`. */
function PickedField({ setting, title, notes, children }: { setting: ExtensionSetting; title: string; notes: ReactNode; children: ReactNode }) {
  return (
    <div className="extension-setting">
      <span className="extension-setting-label" aria-hidden>
        {title}
        {setting.required && <span className="extension-setting-required"> (required)</span>}
      </span>
      {children}
      {notes}
    </div>
  );
}

/** One of a fixed list, with a first entry that clears the choice. */
function Choice({
  label,
  value,
  none,
  options,
  onChange,
}: {
  label: string;
  value: string;
  none: string;
  options: { value: string; label: string }[];
  onChange(value: string): void;
}) {
  const { Combobox } = useContext(ExtensionControls);
  // A saved value the list no longer has is still what is saved: shown, so it can be changed.
  const listed = value && !options.some((option) => option.value === value) ? [{ value, label: value }, ...options] : options;
  return (
    <Combobox
      ariaLabel={label}
      value={value}
      onValueChange={onChange}
      options={[{ value: "", label: none }, ...listed]}
      placeholder={none}
    />
  );
}

/** The kubeconfig contexts, listed when a field needs them. */
function useContexts() {
  return useResource(async () => {
    const outcome = await listContexts(getLiveKubeconfigFiles());
    if (outcome.error && !outcome.contexts) throw new Error(outcome.error);
    return outcome.contexts ?? [];
  }, [], () => false);
}

/**
 * A kubeconfig context, saved by its key (`ClusterContext.key`), the identity
 * app cluster scope uses; its display name changes when another kubeconfig
 * declares the same one (#265).
 */
function ClusterChoice({ label, value, onChange }: { label: string; value: string; onChange(value: string): void }) {
  const contexts = useContexts();
  return (
    <>
      {contexts.status === "error" && (
        <ErrorNotice title="Could not list clusters" message={contexts.error} retry={contexts.reload} />
      )}
      {contexts.status === "loading" && (
        <p role="status" className="extension-setting-note">
          Loading clusters…
        </p>
      )}
      <Choice
        label={label}
        value={value}
        none="Not set"
        options={(contexts.data ?? []).map((context) => ({ value: context.key, label: context.name }))}
        onChange={onChange}
      />
    </>
  );
}

/**
 * A namespace name, picked from the namespaces of a cluster the person
 * chooses: an app setting is app-wide, so which cluster to list is theirs to
 * say. A listing that fails says why and offers a retry; it is never drawn as
 * a cluster with no namespaces.
 */
function NamespaceChoice({ label, value, onChange }: { label: string; value: string; onChange(value: string): void }) {
  const { Combobox } = useContext(ExtensionControls);
  const contexts = useContexts();
  const [from, setFrom] = useState("");
  const namespaces = useResource(async () => {
    if (!from) return null;
    const outcome = await listNamespaces(from);
    if ("error" in outcome && outcome.error) throw new Error(outcome.error);
    return "namespaces" in outcome ? outcome.namespaces : [];
  }, [from], () => false);
  return (
    <>
      {contexts.status === "error" && (
        <ErrorNotice title="Could not list clusters" message={contexts.error} retry={contexts.reload} />
      )}
      <div className="extension-setting-pair">
        <Combobox
          ariaLabel="Cluster to list namespaces from"
          value={from}
          onValueChange={setFrom}
          options={(contexts.data ?? []).map((context) => ({ value: context.stableId, label: context.name }))}
          placeholder={contexts.status === "loading" ? "Loading clusters…" : "List namespaces from…"}
          searchPlaceholder="Find a cluster…"
        />
        <Choice
          label={label}
          value={value}
          none="Not set"
          options={(namespaces.data ?? []).map((namespace) => ({ value: namespace, label: namespace }))}
          onChange={onChange}
        />
      </div>
      {from && namespaces.status === "loading" && (
        <p role="status" className="extension-setting-note">
          Loading namespaces…
        </p>
      )}
      {from && namespaces.status === "error" && (
        <ErrorNotice title="Could not list namespaces" message={namespaces.error} retry={namespaces.reload} cluster />
      )}
      {!from && <p className="extension-setting-note">Choose a cluster to list its namespaces.</p>}
    </>
  );
}
