import { useEffect, useState } from "react";
import { promptIssues, type PromptIssue } from "@srelens/core";

interface McpPromptIssuesProps {
  /**
   * Bumped by the parent to force a re-read. Editing a prompt file takes
   * effect without restarting srelens, but a panel that only fetches on
   * mount would keep showing a stale error after the user fixes their file
   * — the one workflow this panel exists for. Driven by the same Refresh
   * affordance `McpAuditList` already uses for the same reason.
   */
  nonce?: number;
}

/**
 * Prompt files that failed to load. Renders nothing when there are none — the
 * normal case shouldn't occupy space in Settings, but a file that silently
 * failed to appear is the single most confusing thing about authoring prompts.
 */
export function McpPromptIssues({ nonce = 0 }: McpPromptIssuesProps = {}) {
  const [issues, setIssues] = useState<PromptIssue[]>([]);

  useEffect(() => {
    let active = true;
    void promptIssues().then((out) => {
      if (active) setIssues(out);
    });
    return () => {
      active = false;
    };
  }, [nonce]);

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
