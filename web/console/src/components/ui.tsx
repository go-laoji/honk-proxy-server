import type { ComponentType, ReactNode } from "react";

export function Badge({
  children,
  active = false,
}: {
  children: ReactNode;
  active?: boolean;
}) {
  return (
    <span
      className={`inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 text-xs font-medium ${active ? "bg-emerald-50 text-emerald-700 dark:bg-emerald-950 dark:text-emerald-300" : "bg-slate-100 text-slate-600 dark:bg-slate-800 dark:text-slate-300"}`}
    >
      <span
        className={`size-1.5 rounded-full ${active ? "bg-emerald-500" : "bg-slate-400"}`}
      />
      {children}
    </span>
  );
}

export function Stat({
  title,
  value,
  detail,
  icon: Icon,
}: {
  title: string;
  value: string;
  detail: string;
  icon: ComponentType<{ size?: number; className?: string }>;
}) {
  return (
    <div className="panel p-5">
      <div className="flex items-center justify-between">
        <p className="muted text-sm">{title}</p>
        <Icon size={18} className="text-slate-400" />
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
