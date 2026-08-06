import { useEffect, useState } from "react";
import { promptIssues, type PromptIssue } from "../lib/mcpSecurity";

/**
 * Prompt files that failed to load. Renders nothing when there are none — the
 * normal case shouldn't occupy space in Settings, but a file that silently
 * failed to appear is the single most confusing thing about authoring prompts.
 */
export function McpPromptIssues() {
  const [issues, setIssues] = useState<PromptIssue[]>([]);

  useEffect(() => {
    let active = true;
    // `promptIssues()` already swallows the underlying command's errors, but
    // don't rely solely on that contract holding across every call site (e.g.
    // a test that mocks this module wholesale) — an unguarded rejection here
    // would surface as an unhandled promise rejection instead of empty state.
    void promptIssues()
      .then((out) => {
        if (active) setIssues(out);
      })
      .catch(() => {});
    return () => {
      active = false;
    };
  }, []);

  if (issues.length === 0) return null;

  return (
    <div className="flex flex-col gap-1">
      <p className="m-0 text-sm text-amber-600 dark:text-amber-500">
        {issues.length} prompt file{issues.length === 1 ? "" : "s"} could not be loaded:
      </p>
      <ul className="m-0 flex flex-col gap-1 pl-4 text-xs text-muted-foreground">
        {issues.map((issue) => (
          <li key={`${issue.file}:${issue.problem}`}>
            <code className="fl-mono">{issue.file}</code> — {issue.problem}
          </li>
        ))}
      </ul>
    </div>
  );
}
