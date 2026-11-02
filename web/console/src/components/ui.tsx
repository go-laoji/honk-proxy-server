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
  tone = "brand",
}: {
  title: string;
  value: string;
  detail: string;
  icon: ComponentType<{ size?: number; className?: string }>;
  tone?: "brand" | "blue" | "neutral";
}) {
  return (
    <div className="panel stat-card p-5" data-tone={tone}>
      <div className="flex items-center justify-between">
        <p className="muted text-sm">{title}</p>
        <span className="stat-icon" aria-hidden="true">
          <Icon size={18} />
        </span>
      </div>
      <p className="mt-4 text-2xl font-semibold tracking-tight">{value}</p>
      <p className="muted mt-2 text-xs leading-5">{detail}</p>
    </div>
  );
}

export function Details({ rows }: { rows: [string, ReactNode][] }) {
  return (
    <dl className="divide-y divide-slate-100 dark:divide-slate-800">
      {rows.map(([label, value]) => (
        <div
          key={label}
          className="flex flex-col gap-1 px-5 py-4 text-sm sm:flex-row sm:justify-between sm:gap-6"
        >
          <dt className="muted shrink-0">{label}</dt>
          <dd className="break-all sm:text-right">{value}</dd>
        </div>
      ))}
    </dl>
  );
}
