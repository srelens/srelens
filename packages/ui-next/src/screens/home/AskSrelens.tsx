import { useState } from "react";
import { isTauri, type ClusterContext } from "@srelens/core";
import { Button, Eyebrow, TextInput } from "@srelens/ui-kit";
import { attentionQuestion, rankedAttention, type ClusterAttention } from "../../lib/attention";
import { useAskAssistant } from "./ask";

/** Suggestions are a nudge, not a second list of what needs attention. */
const SUGGESTED = 3;

/**
 * "Ask srelens": a prompt that opens the assistant with what was typed, and
 * up to three questions made from the worst of what needs attention. Each
 * suggestion opens on the cluster it is about; a typed question goes to the
 * cluster in focus, as the dock's own prompt does.
 *
 * Not drawn on the web host, where the assistant cannot run.
 */
export function AskSrelens({ targets, scans }: {
  targets: readonly ClusterContext[];
  scans: Readonly<Record<string, ClusterAttention>>;
}) {
  const ask = useAskAssistant();
  const [question, setQuestion] = useState("");
  if (!isTauri()) return null;
  const byId = new Map(targets.map((t) => [t.stableId, t]));
  const suggestions = rankedAttention(targets, scans).slice(0, SUGGESTED);
  const submit = () => {
    const text = question.trim();
    if (!text) return;
    ask(text);
    setQuestion("");
  };

  return (
    <section className="home-side-section" aria-labelledby="home-ask-title">
      <h2 id="home-ask-title" className="home-section-heading">Ask srelens</h2>
      <div className="home-side-group">
        <div className="home-ask-row">
          <TextInput aria-label="Ask srelens" placeholder="Ask about your clusters…" value={question} onValueChange={setQuestion} onEnter={submit} />
          <Button variant="primary" size="sm" disabled={question.trim() === ""} onClick={submit}>Ask</Button>
        </div>
        {suggestions.length > 0 && (
          <>
            <Eyebrow className="mt-3">Suggested</Eyebrow>
            <ul className="home-pick-list">
              {suggestions.map((item) => {
                const text = attentionQuestion(item);
                return (
                  <li key={`${item.clusterId}/${item.kind}/${item.namespace}/${item.name}`}>
                    <button type="button" className="home-pick-row" aria-label={`Ask: ${text}`} onClick={() => ask(text, byId.get(item.clusterId))}>
                      <span className="min-w-0">{text}</span>
                    </button>
                  </li>
                );
              })}
            </ul>
          </>
        )}
      </div>
    </section>
  );
}
