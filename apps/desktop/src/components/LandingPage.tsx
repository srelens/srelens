import React, { useEffect, useMemo, useState } from "react";
import { ArrowRight, FileCode2, Layers3, Search, Settings, SquareTerminal } from "lucide-react";
import srelensMark from "../assets/srelens-mark.svg";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { InputGroup, InputGroupAddon, InputGroupInput } from "@/components/ui/input-group";
import { listContexts, type ClusterContext } from "@srelens/core";
import { ContextAvatar } from "./ContextAvatar";
import type { SettingsSection } from "./SettingsView";
import { contextDisplayName, orderContexts, type ContextProfiles } from "@srelens/core";
import { loadOnboarded, saveOnboarded, shouldShowFirstRun } from "@srelens/core";

const EMPTY_LIST: string[] = [];

const workflowItems = [
  {
    icon: Layers3,
    title: "Discover",
    description: "Move through workloads, networking, storage, and access control without changing tools.",
  },
  {
    icon: FileCode2,
    title: "Understand",
    description: "Inspect health, relationships, events, and manifests in one connected workspace.",
  },
  {
    icon: SquareTerminal,
    title: "Operate",
    description: "Keep logs, shells, forwards, and edits beside the resource that started the task.",
  },
];

export function LandingPage({
  onOpenContext,
  onOpenSettings,
  contextProfiles = {},
  kubeconfigFiles = EMPTY_LIST,
  contextOrder = EMPTY_LIST,
  contexts: passedContexts = null,
  contextsError = "",
}: {
  onOpenContext: (context: string) => void;
  onOpenSettings: (section?: SettingsSection) => void;
  contextProfiles?: ContextProfiles;
  kubeconfigFiles?: string[];
  contextOrder?: string[];
  contexts?: ClusterContext[] | null;
  contextsError?: string;
}) {
  const [internalContexts, setInternalContexts] = useState<ClusterContext[] | null>(null);
  const [internalError, setInternalError] = useState("");
  const [query, setQuery] = useState("");

  useEffect(() => {
    if (passedContexts !== null) return;
    let active = true;
    void listContexts(kubeconfigFiles).then((outcome) => {
      if (!active) return;
      setInternalContexts(orderContexts(outcome.contexts ?? [], contextOrder));
      setInternalError(outcome.error ?? "");
    });
    return () => {
      active = false;
    };
  }, [contextOrder, kubeconfigFiles, passedContexts]);

  const contexts = passedContexts !== null
    ? orderContexts(passedContexts, contextOrder)
    : internalContexts;
  const error = passedContexts !== null ? contextsError : internalError;

  const currentContext = contexts?.find((context) => context.isCurrent) ?? contexts?.[0] ?? null;
  const filteredContexts = useMemo(() => {
    const normalized = query.trim().toLowerCase();
    if (!normalized) return contexts ?? [];
    return (contexts ?? []).filter((context) =>
      [context.name, contextDisplayName(context.name, contextProfiles[context.name]), context.cluster, context.server]
        .some((value) => value.toLowerCase().includes(normalized)),
    );
  }, [contextProfiles, contexts, query]);

  const contextCount = contexts?.length ?? 0;
  // First-run help, shown until the user opens a cluster or dismisses it.
  const [onboarded, setOnboarded] = useState(loadOnboarded);
  const dismissFirstRun = () => {
    saveOnboarded();
    setOnboarded(true);
  };
  const firstRun = shouldShowFirstRun(onboarded, contexts === null ? null : contextCount);


  const localContexts = filteredContexts.filter((context) => context.isLocal);
  const remoteContexts = filteredContexts.filter((context) => !context.isLocal);
  // Only split into labelled sections when both kinds are present; a single
  // group renders as a plain list, exactly as before.
  const grouped = localContexts.length > 0 && remoteContexts.length > 0;

  const renderRow = (context: ClusterContext) => (
    <button
      key={context.name}
      type="button"
      className="fl-landing__context-row"
      onClick={() => {
        // Opening a cluster is the thing the first-run card exists to get
        // the user to do, so it retires the card as surely as dismissing it.
        saveOnboarded();
        onOpenContext(context.name);
      }}
      aria-label={`Open context ${context.name}`}
    >
      <span className="fl-landing__context-main">
        <ContextAvatar
          context={context.name}
          profile={contextProfiles[context.name]}
          className="fl-landing__context-list-avatar"
        />
        <span>
          <strong>{contextDisplayName(context.name, contextProfiles[context.name])}</strong>
          <small>{context.cluster}</small>
        </span>
      </span>
      <span className="fl-landing__context-action" aria-hidden="true">
        {context.isLocal && (
          <Badge variant="outline" className="fl-landing__context-badge">
            {context.provider ?? "local"}
          </Badge>
        )}
        {context.isCurrent && <small>Current</small>}
        <ArrowRight />
      </span>
    </button>
  );

  const renderGroup = (label: string, items: ClusterContext[]) => (
    <div className="fl-landing__context-group" key={label}>
      <p className="fl-landing__context-group-label">
        <span>{label}</span>
        <span className="fl-landing__context-group-count">{items.length}</span>
      </p>
      {items.map(renderRow)}
    </div>
  );

  return (
    <div className="fl-landing">
      <div className="fl-landing__frame">
        <header className="fl-landing__masthead">
          <div className="fl-landing__brand">
            <span className="fl-landing__brand-mark" aria-hidden="true">
              <img src={srelensMark} alt="" />
            </span>
            <span>
              <strong>srelens</strong>
              <small>Kubernetes desktop workspace</small>
            </span>
          </div>
          <Button variant="ghost" size="sm" onClick={() => onOpenSettings()} aria-label="Workspace preferences">
            <Settings data-icon="inline-start" />
            Preferences
          </Button>
        </header>

        <main className="fl-landing__workspace">
          <section className="fl-landing__intro" aria-labelledby="landing-title">
            <Badge variant="outline">Local kubeconfig</Badge>
            <div className="fl-landing__copy">
              <p className="fl-landing__eyebrow">Ready when you are</p>
              <h1 id="landing-title">
                Your clusters.
                <span>One workspace.</span>
              </h1>
              <p>
                A pure-Rust Kubernetes UI built for focused investigation and safe, in-context operations.
              </p>
            </div>

            <Card className="fl-landing__current-card" size="sm">
              <CardHeader>
                <CardTitle>Current context</CardTitle>
                <CardDescription>Continue from your active kubeconfig context.</CardDescription>
                {currentContext && (
                  <CardAction>
                    <span className="fl-landing__current-badges">
                      {currentContext.isLocal && (
                        <Badge variant="outline">{currentContext.provider ?? "local"}</Badge>
                      )}
                      <Badge variant="secondary">Current</Badge>
                    </span>
                  </CardAction>
                )}
              </CardHeader>
              <CardContent>
                {currentContext ? (
                  <div className="fl-landing__current-context">
                    <ContextAvatar
                      context={currentContext.name}
                      profile={contextProfiles[currentContext.name]}
                      className="fl-landing__context-glyph"
                    />
                    <span>
                      <strong>{contextDisplayName(currentContext.name, contextProfiles[currentContext.name])}</strong>
                      <small>{currentContext.cluster}</small>
                    </span>
                  </div>
                ) : (
                  <p className="fl-landing__empty">No kube context is currently available.</p>
                )}
              </CardContent>
              <CardFooter className="fl-landing__current-footer">
                <Button
                  onClick={() => {
                    if (!currentContext) return;
                    // Same as picking from the list: the user is in.
                    saveOnboarded();
                    onOpenContext(currentContext.name);
                  }}
                  disabled={!currentContext}
                  aria-label={`Open current context ${currentContext?.name ?? "cluster"}`}
                >
                  Open workspace
                  <ArrowRight data-icon="inline-end" />
                </Button>
                {currentContext?.server && <code>{currentContext.server}</code>}
              </CardFooter>
            </Card>
          </section>

          {/* One column, not two siblings: the workspace is a two-column grid,
              so a third auto-placed child would take the right column and push
              the context list underneath the intro on the left. */}
          <div className="fl-landing__side">
          {firstRun && (
            <Card className="fl-landing__firstrun" aria-labelledby="firstrun-title">
              <CardHeader>
                <CardTitle id="firstrun-title">New here?</CardTitle>
                <CardDescription>
                  Three things worth knowing. This card goes away once you open a cluster.
                </CardDescription>
                <CardAction>
                  <Button variant="ghost" size="sm" onClick={dismissFirstRun}>
                    Dismiss
                  </Button>
                </CardAction>
              </CardHeader>
              <CardContent>
                <ul className="fl-landing__firstrun-list">
                  <li>
                    <strong>Pick a context below</strong> to open that cluster — these come from your
                    kubeconfig.
                  </li>
                  <li>
                    <strong>Cmd/Ctrl-K</strong> jumps to any view or resource by name.
                  </li>
                  <li>
                    <strong>?</strong> lists every keyboard shortcut.
                  </li>
                </ul>
              </CardContent>
            </Card>
          )}

          <Card className="fl-landing__contexts" aria-label="Available contexts">
            <CardHeader>
              <CardTitle>Contexts</CardTitle>
              <CardDescription>Select any context from your local kubeconfig.</CardDescription>
              <CardAction>
                <Badge variant="outline">
                  {contextCount} {contextCount === 1 ? "context" : "contexts"}
                </Badge>
              </CardAction>
            </CardHeader>
            <CardContent className="fl-landing__contexts-content">
              <InputGroup>
                <InputGroupAddon>
                  <Search />
                </InputGroupAddon>
                <InputGroupInput
                  value={query}
                  onChange={(event) => setQuery(event.target.value)}
                  placeholder="Filter contexts"
                  aria-label="Filter contexts"
                />
              </InputGroup>

              <div className="fl-landing__context-list">
                {contexts === null ? (
                  <p className="fl-landing__empty">Reading kubeconfig…</p>
                ) : error ? (
                  <p className="fl-landing__empty">Unable to load kube contexts.</p>
                ) : contextCount === 0 ? (
                  <div className="fl-landing__empty">
                    <p>
                      No clusters yet — srelens read your kubeconfig and found no contexts in it.
                    </p>
                    <Button size="sm" onClick={() => onOpenSettings("contexts")}>
                      Add or paste a kubeconfig
                      <ArrowRight data-icon="inline-end" />
                    </Button>
                  </div>
                ) : filteredContexts.length === 0 ? (
                  <p className="fl-landing__empty">No contexts match “{query}”. Clear the filter to see all {contextCount}.</p>
                ) : grouped ? (
                  <>
                    {renderGroup("Local", localContexts)}
                    {renderGroup("Remote", remoteContexts)}
                  </>
                ) : (
                  filteredContexts.map(renderRow)
                )}
              </div>
            </CardContent>
            <CardFooter className="fl-landing__contexts-footer">
              <span>{filteredContexts.length} shown</span>
              <span>Source: local kubeconfig</span>
            </CardFooter>
          </Card>
          </div>
        </main>

        <section className="fl-landing__capabilities" aria-labelledby="capabilities-title">
          <div className="fl-landing__section-heading">
            <p>Designed for operations</p>
            <h2 id="capabilities-title">Stay with the resource from signal to action.</h2>
          </div>
          <div className="fl-landing__capability-grid">
            {workflowItems.map(({ icon: Icon, title, description }) => (
              <Card key={title} className="fl-landing__capability" size="sm">
                <CardHeader>
                  <span className="fl-landing__capability-icon" aria-hidden="true">
                    <Icon />
                  </span>
                  <CardTitle>{title}</CardTitle>
                  <CardDescription>{description}</CardDescription>
                </CardHeader>
              </Card>
            ))}
          </div>
        </section>
      </div>
    </div>
  );
}
