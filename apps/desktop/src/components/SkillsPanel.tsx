import { useEffect, useRef, useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from "@/components/ui/dialog";
import { Textarea } from "@/components/ui/textarea";
import { Button, ConfirmDialog, Field, TextInput } from "../ui";
import { notify } from "@srelens/core";
import { listSkills, loadSkill, saveSkill, deleteSkill, type Skill, type SkillMeta } from "@srelens/core";
import { cancelChat, listAgents, startChat, sendChat, type AgentEvent, type AgentInfo } from "@srelens/core";

const BLANK: Skill = { name: "", description: "", body: "" };

/**
 * Fixed meta-prompt (Task 23) sent as a single non-conversational turn to
 * generate a skill: name/description front-matter plus a body, for the
 * user's one-sentence need. Output only the markdown, so the whole reply can
 * be dropped straight into the editor.
 */
function buildMetaPrompt(need: string): string {
  return `Write a srelens assistant skill as markdown with name/description front-matter for the following need: ${need}. Output only the markdown.`;
}

/**
 * Pulls a leading `---\nname: ...\ndescription: ...\n---` front-matter block
 * off generated markdown. When the markdown doesn't start with that shape,
 * `name`/`description` come back `null` and `body` is the markdown
 * unchanged — the caller just drops the whole thing into the body field.
 */
export function parseFrontMatter(markdown: string): { name: string | null; description: string | null; body: string } {
  const match = /^---[ \t]*\n([\s\S]*?)\n---[ \t]*\n?([\s\S]*)$/.exec(markdown);
  if (!match) return { name: null, description: null, body: markdown };
  const [, frontMatter, rest] = match;
  const nameMatch = /^name:\s*(.+)$/m.exec(frontMatter);
  const descMatch = /^description:\s*(.+)$/m.exec(frontMatter);
  return {
    name: nameMatch ? nameMatch[1].trim() : null,
    description: descMatch ? descMatch[1].trim() : null,
    body: rest,
  };
}

/**
 * Skills — reusable instruction files the AI assistant can draw on (Task 22;
 * a chat doesn't yet DO anything with a skill it loads — Task 23 wires that
 * in). A simple list-plus-editor modal, opened from the full-tab assistant's
 * history rail (`AssistantTab`): plain name/description inputs and a
 * plain-text body textarea, no rich markdown WYSIWYG, matching how
 * `HelmOpDialog`/`McpConfirmDialog` build a form inside the shared `Dialog`
 * primitive.
 */
export function SkillsPanel({ onClose }: { onClose: () => void }) {
  const [skills, setSkills] = useState<SkillMeta[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [editing, setEditing] = useState<Skill | null>(null);
  const [pendingDelete, setPendingDelete] = useState<string | null>(null);
  const [error, setError] = useState("");
  // Generate-with-AI (Task 23): a one-sentence need, run through a single
  // non-conversational bridge turn, that fills the editor's fields for the
  // user to review before Save — never auto-saved.
  const [agents, setAgents] = useState<AgentInfo[]>([]);
  const [need, setNeed] = useState("");
  const [generating, setGenerating] = useState(false);
  // The in-flight generation's chat session, so closing the panel mid-turn
  // can stop the backend turn (CLI process or native provider task) instead
  // of leaving it running invisibly with no Stop control left anywhere.
  const genSessionRef = useRef<string | null>(null);
  // Set by unmount; `generate` re-checks it after its async prep so a
  // `startChat` that resolves after the panel closed never launches the turn.
  const genCancelledRef = useRef(false);
  useEffect(
    () => () => {
      genCancelledRef.current = true;
      if (genSessionRef.current) void cancelChat(genSessionRef.current);
    },
    [],
  );
  const [generateError, setGenerateError] = useState("");

  async function refresh() {
    try {
      setSkills(await listSkills());
    } catch (e) {
      setError(String(e));
    }
  }

  useEffect(() => {
    void refresh();
  }, []);

  useEffect(() => {
    listAgents()
      .then(setAgents)
      .catch(() => setAgents([]));
  }, []);

  async function select(name: string) {
    setError("");
    setGenerateError("");
    try {
      const skill = await loadSkill(name);
      setSelected(name);
      setEditing(skill);
    } catch (e) {
      setError(String(e));
    }
  }

  function newSkill() {
    setError("");
    setGenerateError("");
    setSelected(null);
    setEditing({ ...BLANK });
  }

  const generateAgent = agents.find((a) => a.available && !a.gated);
  const canGenerate = !!editing && !!generateAgent && need.trim().length > 0 && !generating;

  /**
   * Runs ONE non-conversational turn through the same bridge the chat uses
   * (`startChat`/`sendChat`) with the fixed meta-prompt, accumulates the
   * streamed `textDelta`s, and — only once the turn finishes with no
   * `error` — drops the result into the editor's body (and name/description,
   * if the markdown carries recognizable front-matter). A stream `error` (or
   * a thrown `sendChat`) surfaces inline instead, leaving whatever was
   * already in the editor untouched — never a partial overwrite.
   */
  async function generate() {
    // No `.path` requirement: the native srelens agent runs in-process and
    // lists with `path: null` — `chat_send` never touches the path for it.
    // Requiring one here would silently no-op the enabled Generate button
    // whenever the native agent sorts first.
    if (!editing || !generateAgent || !need.trim() || generating) return;
    setGenerateError("");
    setGenerating(true);
    let accumulated = "";
    let errored = false;
    try {
      const session = await startChat();
      genSessionRef.current = session;
      // The panel closed while `startChat` was in flight — don't launch.
      if (genCancelledRef.current) return;
      await sendChat(session, buildMetaPrompt(need), generateAgent.path ?? "", (e: AgentEvent) => {
        switch (e.type) {
          case "textDelta":
            accumulated += e.text;
            break;
          case "error":
            errored = true;
            setGenerateError(e.message);
            break;
          case "turnDone":
            if (!errored) {
              const parsed = parseFrontMatter(accumulated);
              setEditing((prev) =>
                prev
                  ? {
                      ...prev,
                      body: parsed.body,
                      name: parsed.name ?? prev.name,
                      description: parsed.description ?? prev.description,
                    }
                  : prev,
              );
            }
            break;
        }
      }, [], generateAgent.kind);
    } catch (e) {
      setGenerateError(e instanceof Error ? e.message : String(e));
    } finally {
      genSessionRef.current = null;
      setGenerating(false);
    }
  }

  async function save() {
    if (!editing) return;
    setError("");
    const renamedFrom = selected;
    // Refuse to clobber a DIFFERENT existing skill — whether creating a new one
    // or renaming onto another's name. Without this, `saveSkill` would overwrite
    // that file and the rename-cleanup below would then delete the source,
    // silently destroying the target skill.
    if (skills.some((s) => s.name === editing.name && s.name !== renamedFrom)) {
      setError(`A skill named "${editing.name}" already exists — choose a different name.`);
      return;
    }
    try {
      await saveSkill(editing);
      // Renaming an existing skill writes `<new>.md` but the old `<selected>.md`
      // would linger — both would show in the list and the stale one stay
      // selectable. Remove it once the new file is safely written.
      if (renamedFrom && renamedFrom !== editing.name) {
        await deleteSkill(renamedFrom);
      }
      setSelected(editing.name);
      await refresh();
      notify.success("Skill saved");
    } catch (e) {
      setError(String(e));
    }
  }

  async function confirmDelete() {
    const name = pendingDelete;
    if (!name) return;
    setPendingDelete(null);
    try {
      await deleteSkill(name);
      if (selected === name) {
        setSelected(null);
        setEditing(null);
      }
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  }

  return (
    <>
      <Dialog
        open
        onOpenChange={(open) => {
          if (!open) onClose();
        }}
      >
        <DialogContent className="sm:max-w-3xl">
          <DialogHeader>
            <DialogTitle>Skills</DialogTitle>
            <DialogDescription className="sr-only">
              Reusable instruction files the AI assistant can draw on.
            </DialogDescription>
          </DialogHeader>

          <div className="flex h-[28rem] min-h-0 gap-4">
            <div className="flex w-56 shrink-0 flex-col gap-2 border-r border-border pr-3">
              <Button variant="secondary" size="sm" onClick={newSkill} disabled={generating}>
                New skill
              </Button>
              {skills.length === 0 ? (
                <p className="px-1 text-xs text-muted-foreground">No skills yet.</p>
              ) : (
                <ul className="min-h-0 flex-1 space-y-0.5 overflow-y-auto">
                  {skills.map((s) => (
                    <li
                      key={s.name}
                      className="group flex items-center gap-1 rounded-md px-2 py-1.5 text-sm hover:bg-accent"
                    >
                      <button
                        type="button"
                        className="min-w-0 flex-1 truncate text-left disabled:pointer-events-none disabled:opacity-50"
                        onClick={() => void select(s.name)}
                        disabled={generating}
                      >
                        <span className="flex items-center gap-1.5">
                          <span className="min-w-0 truncate font-medium">{s.name}</span>
                          {s.builtin && (
                            <span className="shrink-0 rounded border border-border px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
                              Default
                            </span>
                          )}
                        </span>
                        <span className="block truncate text-xs text-muted-foreground">{s.description}</span>
                      </button>
                      {!s.builtin && (
                        <button
                          type="button"
                          aria-label={`Delete ${s.name}`}
                          onClick={() => setPendingDelete(s.name)}
                          disabled={generating}
                          className="shrink-0 text-muted-foreground opacity-0 hover:text-foreground focus-visible:opacity-100 group-hover:opacity-100 disabled:pointer-events-none disabled:opacity-0"
                        >
                          ✕
                        </button>
                      )}
                    </li>
                  ))}
                </ul>
              )}
            </div>

            <div className="flex min-w-0 flex-1 flex-col gap-3">
              {editing ? (
                <>
                  <div className="flex flex-col gap-2 rounded-md border border-border bg-muted/20 p-3">
                    <Field label="Generate with AI — describe the need">
                      <TextInput
                        aria-label="Skill need"
                        value={need}
                        onValueChange={setNeed}
                        placeholder="Triage a pod that keeps restarting"
                        disabled={generating}
                      />
                    </Field>
                    <div className="flex items-center gap-2">
                      <Button variant="secondary" size="sm" onClick={() => void generate()} disabled={!canGenerate}>
                        {generating ? "Generating…" : "Generate with AI"}
                      </Button>
                      {!generateAgent && (
                        <span className="text-xs text-muted-foreground">Install/enable an agent to generate</span>
                      )}
                    </div>
                    {generateError && (
                      <p role="alert" className="text-xs text-destructive">
                        {generateError}
                      </p>
                    )}
                  </div>
                  <Field label="Name">
                    <TextInput
                      aria-label="Skill name"
                      value={editing.name}
                      onValueChange={(v) => setEditing({ ...editing, name: v })}
                      placeholder="crashloop-triage"
                    />
                  </Field>
                  <Field label="Description">
                    <TextInput
                      aria-label="Skill description"
                      value={editing.description}
                      onValueChange={(v) => setEditing({ ...editing, description: v })}
                      placeholder="Systematic triage for a pod that keeps restarting"
                    />
                  </Field>
                  <Field label="Body" className="min-h-0 flex-1">
                    <Textarea
                      aria-label="Skill body"
                      value={editing.body}
                      onChange={(e) => setEditing({ ...editing, body: e.target.value })}
                      className="h-full min-h-[12rem] resize-none font-mono text-xs"
                    />
                  </Field>
                  <div className="flex justify-end">
                    <Button variant="primary" onClick={() => void save()} disabled={!editing.name.trim()}>
                      Save
                    </Button>
                  </div>
                </>
              ) : (
                <p className="text-sm text-muted-foreground">Select a skill, or create a new one.</p>
              )}
              {error && (
                <p role="alert" className="text-sm text-destructive">
                  {error}
                </p>
              )}
            </div>
          </div>
        </DialogContent>
      </Dialog>

      {pendingDelete && (
        <ConfirmDialog
          title={`Delete "${pendingDelete}"?`}
          message="This removes the skill's file from disk. This cannot be undone."
          confirmLabel="Delete"
          danger
          onConfirm={() => void confirmDelete()}
          onCancel={() => setPendingDelete(null)}
        />
      )}
    </>
  );
}
