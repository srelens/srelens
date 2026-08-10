import { useEffect, useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from "@/components/ui/dialog";
import { Textarea } from "@/components/ui/textarea";
import { Button, ConfirmDialog, Field, TextInput } from "../ui";
import { notify } from "../lib/notify";
import { listSkills, loadSkill, saveSkill, deleteSkill, type Skill, type SkillMeta } from "../lib/skills";

const BLANK: Skill = { name: "", description: "", body: "" };

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

  async function select(name: string) {
    setError("");
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
    setSelected(null);
    setEditing({ ...BLANK });
  }

  async function save() {
    if (!editing) return;
    setError("");
    try {
      await saveSkill(editing);
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
              <Button variant="secondary" size="sm" onClick={newSkill}>
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
                        className="min-w-0 flex-1 truncate text-left"
                        onClick={() => void select(s.name)}
                      >
                        <span className="block truncate font-medium">{s.name}</span>
                        <span className="block truncate text-xs text-muted-foreground">{s.description}</span>
                      </button>
                      <button
                        type="button"
                        aria-label={`Delete ${s.name}`}
                        onClick={() => setPendingDelete(s.name)}
                        className="shrink-0 text-muted-foreground opacity-0 hover:text-foreground focus-visible:opacity-100 group-hover:opacity-100"
                      >
                        ✕
                      </button>
                    </li>
                  ))}
                </ul>
              )}
            </div>

            <div className="flex min-w-0 flex-1 flex-col gap-3">
              {editing ? (
                <>
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
