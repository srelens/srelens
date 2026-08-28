import React from "react";
import { AlertTriangle } from "lucide-react";
import { cn } from "@/ui/utils";
import { Button } from "./Button";

export type DashboardTone = "primary" | "success" | "warning" | "danger" | "info" | "neutral";

export function PageShell({ children, className }: { children: React.ReactNode; className?: string }) {
  return <div className={cn("fl-page-shell", className)}>{children}</div>;
}

export function PageHeader({
  eyebrow,
  title,
  description,
  actions,
}: {
  eyebrow?: React.ReactNode;
  title: React.ReactNode;
  description?: React.ReactNode;
  actions?: React.ReactNode;
}) {
  return (
    <header className="fl-page-header">
      <div className="fl-page-header__copy">
        {eyebrow && <p className="fl-page-header__eyebrow">{eyebrow}</p>}
        <h1>{title}</h1>
        {description && <p>{description}</p>}
      </div>
      {actions && <div className="fl-page-header__actions">{actions}</div>}
    </header>
  );
}

export function SectionPanel({
  title,
  description,
  children,
  className,
}: {
  title?: React.ReactNode;
  description?: React.ReactNode;
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <section className={cn("fl-section-panel", className)}>
      {(title || description) && (
        <header className="fl-section-panel__header">
          {title && <h2>{title}</h2>}
          {description && <p>{description}</p>}
        </header>
      )}
      <div className="fl-section-panel__body">{children}</div>
    </section>
  );
}

export function MetricTile({
  label,
  value,
  description,
  tone = "neutral",
  action,
}: {
  label: React.ReactNode;
  value: React.ReactNode;
  description?: React.ReactNode;
  tone?: DashboardTone;
  action?: React.ReactNode;
}) {
  return (
    <article className={cn("fl-metric-tile", `fl-tone-${tone}`)}>
      <div className="fl-metric-tile__main">
        <p className="fl-metric-tile__label">{label}</p>
        <strong className="fl-metric-tile__value">{value}</strong>
        {description && <p className="fl-metric-tile__description">{description}</p>}
      </div>
      {action && <div className="fl-metric-tile__action">{action}</div>}
    </article>
  );
}

export function StatusMeter({
  label,
  value,
  detail,
  tone = "primary",
}: {
  label: React.ReactNode;
  value: number;
  detail?: React.ReactNode;
  tone?: DashboardTone;
}) {
  const bounded = Math.max(0, Math.min(100, value));
  return (
    <div className={cn("fl-status-meter", `fl-tone-${tone}`)}>
      <div className="fl-status-meter__header">
        <span>{label}</span>
        <strong>{bounded.toFixed(0)}%</strong>
      </div>
      <div className="fl-status-meter__track">
        <span style={{ width: `${bounded}%` }} />
      </div>
      {detail && <p className="fl-status-meter__detail">{detail}</p>}
    </div>
  );
}

export function Toolbar({ children, className }: { children: React.ReactNode; className?: string }) {
  return <div className={cn("fl-toolbar", className)}>{children}</div>;
}

export function EmptyState({ title, description }: { title: React.ReactNode; description?: React.ReactNode }) {
  return (
    <div className="fl-empty-state">
      <strong>{title}</strong>
      {description && <p>{description}</p>}
    </div>
  );
}

/** Error card for a failed load — a clear title, an actionable detail, and an optional retry. */
export function ErrorState({
  title,
  detail,
  onRetry,
  retryLabel = "Retry",
  action,
}: {
  title: React.ReactNode;
  detail?: React.ReactNode;
  onRetry?: () => void;
  retryLabel?: string;
  /** An optional secondary action (e.g. "Diagnose in Toolbox"). */
  action?: { label: string; onClick: () => void };
}) {
  return (
    <div className="fl-error-state" role="alert">
      <AlertTriangle className="fl-error-state__icon" aria-hidden />
      <strong className="fl-error-state__title">{title}</strong>
      {detail && <p className="fl-error-state__detail">{detail}</p>}
      {(onRetry || action) && (
        <div className="fl-error-state__actions">
          {onRetry && (
            <Button variant="secondary" size="sm" onClick={onRetry}>
              {retryLabel}
            </Button>
          )}
          {action && (
            <Button variant="secondary" size="sm" onClick={action.onClick}>
              {action.label}
            </Button>
          )}
        </div>
      )}
    </div>
  );
}

// Backward-compatible aliases for older imports while surfaces are migrated.
export const DashboardPage = PageShell;
export const DashboardHero = SectionPanel;
export const DashboardCard = MetricTile;
export const DashboardMeter = StatusMeter;
export const DashboardChip = MetricTile;
export function DashboardSegmentBar({
  segments,
}: {
  segments: Array<{ value: number; tone: DashboardTone; label: string }>;
}) {
  const total = segments.reduce((sum, segment) => sum + Math.max(0, segment.value), 0);
  return (
    <div className="fl-segment-bar" aria-label="Segmented status bar">
      {segments.map((segment) => {
        const width = total > 0 ? (segment.value / total) * 100 : 0;
        return (
          <span
            key={segment.label}
            className={cn("fl-segment-bar__item", `fl-tone-${segment.tone}`)}
            style={{ width: `${width}%` }}
            title={`${segment.label}: ${segment.value}`}
          />
        );
      })}
    </div>
  );
}
