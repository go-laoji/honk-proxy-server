import { useState, type ReactNode } from "react";
import { ChevronDown } from "lucide-react";

export function AdvancedFilters({
  active,
  children,
}: {
  active: boolean;
  children: ReactNode;
}) {
  return (
    <details className="group" open={active}>
      <summary className="text-link w-fit list-none rounded-sm">
        <ChevronDown
          size={14}
          className="-rotate-90 group-open:rotate-0"
          aria-hidden="true"
        />
        高级筛选
      </summary>
      <div className="filter-grid mt-2">{children}</div>
    </details>
  );
}

function localDateTime(timestamp: string) {
  const date = new Date(Number(timestamp));
  if (!timestamp || !Number.isFinite(date.getTime())) return "";
  date.setTime(date.getTime() - date.getTimezoneOffset() * 60_000);
  return Number.isFinite(date.getTime()) ? date.toISOString().slice(0, 23) : "";
}

export function FilterStartTime({
  name,
  value,
}: {
  name: string;
  value: string;
}) {
  const [timestamp, setTimestamp] = useState(value);
  return (
    <label className="form-field col-span-2 sm:col-span-1">
      起始时间（本地）
      <input type="hidden" name={name} value={timestamp} />
      <input
        className="field"
        type="datetime-local"
        step="0.001"
        min={localDateTime("0")}
        defaultValue={localDateTime(value)}
        onChange={(event) => {
          const time = new Date(event.target.value).getTime();
          setTimestamp(Number.isFinite(time) ? String(time) : "");
        }}
      />
    </label>
  );
}
