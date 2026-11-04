import type { ComponentType, ReactNode } from "react";

export function Badge({
  children,
  active = false,
}: {
  children: ReactNode;
  active?: boolean;
}) {
  return (
    <span className="status-badge" data-active={active}>
      <span className="status-dot" aria-hidden="true" />
      {children}
    </span>
  );
}

export function Stat({
  title,
  value,
  detail,
  icon: Icon,
  tone,
}: {
  title: string;
  value: string;
  detail: string;
  icon: ComponentType<{ size?: number; className?: string }>;
  tone?: "down" | "up" | "active" | "failed";
}) {
  return (
    <div className="panel panel-body stat-card min-w-0" data-tone={tone}>
      <div className="flex items-center gap-2">
        <span className="stat-icon" aria-hidden="true">
          <Icon size={15} />
        </span>
        <p className="muted text-xs font-medium">{title}</p>
      </div>
      <p className="mt-1.5 text-xl font-semibold tracking-tight tabular-nums sm:text-2xl">
        {value}
      </p>
      <p className="muted mt-0.5 text-xs leading-5">{detail}</p>
    </div>
  );
}

export function Details({ rows }: { rows: [string, ReactNode][] }) {
  return (
    <dl className="divide-y divide-console-border">
      {rows.map(([label, value]) => (
        <div
          key={label}
          className="flex flex-col gap-1 px-4 py-2.5 text-sm sm:flex-row sm:justify-between sm:gap-4"
        >
          <dt className="muted shrink-0">{label}</dt>
          <dd className="break-all sm:text-right">{value}</dd>
        </div>
      ))}
    </dl>
  );
}
