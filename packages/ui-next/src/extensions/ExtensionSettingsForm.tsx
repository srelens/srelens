import { useContext, useId, useState, type ReactNode } from "react";
import {
  getLiveKubeconfigFiles,
  listContexts,
  listNamespaces,
  type ExtensionSecretStoreState,
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
 * text and is drawn as plain text. A secret is never one of these settings:
 * its field writes straight to the host's secret store (#543) and is
 * write-only — set, replace, clear, and whether it is set.
 */
export function ExtensionSettingsForm({
  plugin,
  secretStore,
  onSave,
  onSetSecret,
  onClearSecret,
  onClose,
}: {
  plugin: InstalledExtension;
  /** Whether the host can keep a secret now, from `extensions.list`. Absent is treated as no. */
  secretStore?: ExtensionSecretStoreState;
  /** Saves through the host, which checks every value; rejects with the host's reason. */
  onSave(settings: Record<string, unknown>): Promise<void>;
  /** Keeps a secret in the host's store; rejects with the host's reason, which never holds the value. */
  onSetSecret?(setting: string, secret: string): Promise<void>;
  /** Deletes a secret from the host's store. */
  onClearSecret?(setting: string): Promise<void>;
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
          {declared.map((setting) =>
            isSecret(setting) ? (
              <SecretField
                key={setting.id}
                setting={setting}
                saved={plugin.settings[setting.id]}
                refusal={secretRefusal(plugin, secretStore, onSetSecret)}
                onSet={onSetSecret && ((secret) => onSetSecret(setting.id, secret))}
                onClear={onClearSecret && (() => onClearSecret(setting.id))}
              />
            ) : (
              <SettingField
                key={setting.id}
                setting={setting}
                value={draft[setting.id]}
                saved={plugin.settings[setting.id]}
                problem={problems[setting.id]}
                onChange={(value) => set(setting.id, value)}
              />
            ),
          )}
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
  // What a picker announces, the same as the text inputs': a picker is
  // labelled by its title, so the visible "(required)" is not what says so.
  const pickerState: PickerState = {
    ariaInvalid: invalid,
    ariaRequired: setting.required || undefined,
    ariaDescribedBy: described,
  };

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
            state={pickerState}
          />
        </PickedField>
      );
    case "cluster-selector":
      return (
        <PickedField setting={setting} title={title} notes={notes}>
          <ClusterChoice label={title} value={text} onChange={onChange} state={pickerState} />
        </PickedField>
      );
    case "namespace-selector":
      return (
        <fieldset className="extension-setting" aria-invalid={invalid} aria-describedby={described}>
          <legend>
            {title}
            {setting.required && <span className="extension-setting-required"> (required)</span>}
          </legend>
          <NamespaceChoice label={title} value={text} onChange={onChange} state={pickerState} />
          {notes}
        </fieldset>
      );
    case "secret-reference":
      // Drawn by `SecretField`; a secret never reaches this form's draft.
      return null;
  }
}

/** The permission an app needs for the host to keep its secrets (#543). */
const SECRET_STORE_PERMISSION = "extension.secretStore";

/**
 * Why no secret can be set for this app now, or `undefined` when one can. The
 * host checks all of it again; this only keeps the form from offering what
 * the host will refuse, and says why. Fails closed: a host that does not
 * report its store is taken to have none.
 */
function secretRefusal(
  plugin: InstalledExtension,
  store: ExtensionSecretStoreState | undefined,
  onSet: unknown,
): string | undefined {
  if (!plugin.grants.includes(SECRET_STORE_PERMISSION))
    return `This app was not granted ${SECRET_STORE_PERMISSION}, so it cannot keep secrets. Reinstall it to review its permissions.`;
  if (!onSet || !store) return "The host did not say whether it can keep a secret, so none can be set.";
  if (!store.available)
    return `${store.reason ?? "The host cannot keep a secret right now"}. Nothing is saved in plain text.`;
  return undefined;
}

/**
 * One `secret-reference` setting: whether it is set, and a write-only
 * password field. The typed value goes to the host's store and is dropped
 * from the page as soon as it is sent, whatever the answer; nothing ever
 * shows it again.
 */
function SecretField({
  setting,
  saved,
  refusal,
  onSet,
  onClear,
}: {
  setting: ExtensionSetting;
  saved: unknown;
  /** Why a secret cannot be set now; the field is disabled and says so. */
  refusal?: string;
  onSet?(secret: string): Promise<void>;
  onClear?(): Promise<void>;
}) {
  const { Button } = useContext(ExtensionControls);
  const id = useId();
  const [typed, setTyped] = useState("");
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState("");
  const [failure, setFailure] = useState("");
  const title = plainText(setting.title);
  const help = setting.description ? plainText(setting.description) : undefined;
  const isSet = typeof saved === "object" && saved !== null && "secretRef" in saved;
  const described = [help && `${id}-help`, `${id}-state`, refusal && `${id}-refusal`, failure && `${id}-problem`]
    .filter(Boolean)
    .join(" ");

  async function run(action: () => Promise<void>, success: string) {
    setBusy(true);
    setDone("");
    setFailure("");
    try {
      await action();
      setDone(success);
    } catch (e) {
      setFailure(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }
  function save() {
    if (!onSet || refusal || !typed) return;
    const secret = typed;
    // Out of the page before the answer: kept or refused, it is not shown again.
    setTyped("");
    void run(() => onSet(secret), "Secret saved.");
  }

  return (
    <fieldset className="extension-setting" aria-describedby={described}>
      <legend id={`${id}-title`}>{title}</legend>
      {help && (
        <p id={`${id}-help`} className="extension-setting-note">
          {help}
        </p>
      )}
      <p id={`${id}-state`} className="extension-setting-note">
        <strong>{isSet ? "Set" : "Not set"}</strong>
        {" · "}
        Kept in srelens's encrypted secrets vault. It is never shown again, and never saved in these settings or in exported settings.
      </p>
      {refusal && (
        <p id={`${id}-refusal`} className="extension-setting-problem">
          {refusal}
        </p>
      )}
      <div className="extension-setting-pair">
        <input
          id={id}
          aria-labelledby={`${id}-title`}
          type="password"
          autoComplete="new-password"
          spellCheck={false}
          value={typed}
          disabled={Boolean(refusal) || busy}
          placeholder={isSet ? "Enter a new value to replace it" : "Enter the secret"}
          aria-invalid={failure ? true : undefined}
          aria-describedby={described}
          onChange={(event) => {
            setTyped(event.target.value);
            setDone("");
          }}
          onKeyDown={(event) => {
            // Enter keeps this secret; it does not submit the other settings.
            if (event.key === "Enter") {
              event.preventDefault();
              save();
            }
          }}
        />
        <Button type="button" disabled={Boolean(refusal) || busy || !typed} onClick={save}>
          {isSet ? "Replace secret" : "Save secret"}
        </Button>
        {isSet && onClear && (
          <Button type="button" variant="secondary" disabled={busy} onClick={() => void run(onClear, "Secret cleared.")}>
            Clear secret
          </Button>
        )}
      </div>
      {failure && (
        <p id={`${id}-problem`} role="alert" className="extension-setting-problem">
          {failure}
        </p>
      )}
      {done && (
        <p role="status" className="extension-settings-saved">
          {done}
        </p>
      )}
    </fieldset>
  );
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

/** A picker's form-field state, announced as a text input's is. */
interface PickerState {
  ariaInvalid?: boolean;
  ariaRequired?: boolean;
  ariaDescribedBy?: string;
}

/** One of a fixed list, with a first entry that clears the choice. */
function Choice({
  label,
  value,
  none,
  options,
  onChange,
  state,
}: {
  label: string;
  value: string;
  none: string;
  options: { value: string; label: string }[];
  onChange(value: string): void;
  state?: PickerState;
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
      {...state}
    />
  );
}

/**
 * The kubeconfig contexts, listed when a field needs them. `listContexts`
 * answers with the contexts it could read AND why the rest are missing when
 * some kubeconfig files fail; both are kept, so a partial list is never drawn
 * as the whole one.
 */
function useContexts() {
  return useResource(async () => {
    const outcome = await listContexts(getLiveKubeconfigFiles());
    if (outcome.error && !outcome.contexts) throw new Error(outcome.error);
    return { contexts: outcome.contexts ?? [], partial: outcome.error || undefined };
  }, [], () => false);
}

/** Why the clusters could not be listed, whole or in part. */
function ContextsProblem({ contexts }: { contexts: ReturnType<typeof useContexts> }) {
  if (contexts.status === "error")
    return <ErrorNotice title="Could not list clusters" message={contexts.error} retry={contexts.reload} />;
  if (contexts.data?.partial)
    return (
      <ErrorNotice
        title="Some clusters could not be listed"
        message={contexts.data.partial}
        retry={contexts.reload}
      />
    );
  return null;
}

/**
 * A kubeconfig context, saved by its key (`ClusterContext.key`), the identity
 * app cluster scope uses; its display name changes when another kubeconfig
 * declares the same one (#265).
 */
function ClusterChoice({
  label,
  value,
  onChange,
  state,
}: {
  label: string;
  value: string;
  onChange(value: string): void;
  state?: PickerState;
}) {
  const contexts = useContexts();
  return (
    <>
      <ContextsProblem contexts={contexts} />
      {contexts.status === "loading" && (
        <p role="status" className="extension-setting-note">
          Loading clusters…
        </p>
      )}
      <Choice
        label={label}
        value={value}
        none="Not set"
        options={(contexts.data?.contexts ?? []).map((context) => ({ value: context.key, label: context.name }))}
        onChange={onChange}
        state={state}
      />
    </>
  );
}

/**
 * A namespace name, picked from the namespaces of a cluster the person
 * chooses: an app setting is app-wide, so which cluster to list is theirs to
 * say. A listing that fails says why and offers a retry; it is never drawn as
 * a cluster with no namespaces.
 *
 * The cluster is sent to `listNamespaces` by its name, as every other caller
 * of core's `list*` does: the backend resolves a name, a stable ID or a pinned
 * ID (`find_context`), never a key, and a stable ID two contexts share
 * resolves to neither.
 */
function NamespaceChoice({
  label,
  value,
  onChange,
  state,
}: {
  label: string;
  value: string;
  onChange(value: string): void;
  state?: PickerState;
}) {
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
      <ContextsProblem contexts={contexts} />
      <div className="extension-setting-pair">
        <Combobox
          ariaLabel="Cluster to list namespaces from"
          value={from}
          onValueChange={setFrom}
          options={(contexts.data?.contexts ?? []).map((context) => ({ value: context.name, label: context.name }))}
          placeholder={contexts.status === "loading" ? "Loading clusters…" : "List namespaces from…"}
          searchPlaceholder="Find a cluster…"
        />
        <Choice
          label={label}
          value={value}
          none="Not set"
          options={(namespaces.data ?? []).map((namespace) => ({ value: namespace, label: namespace }))}
          onChange={onChange}
          state={state}
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
