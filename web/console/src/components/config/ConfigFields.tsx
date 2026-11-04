import { useId, useState, type ReactNode } from "react";
import { Minus, Plus } from "lucide-react";

type RecordValue = Record<string, unknown>;
type Choices = Readonly<Record<string, string>>;
type NamedField = {
  key: string;
  label: string;
  help?: string;
  when?: (value: RecordValue) => boolean;
};
type TextField = NamedField & {
  kind: "text" | "url" | "password";
  required?: boolean;
  readOnly?: boolean;
  nullable?: boolean;
  maxLength?: number;
  pattern?: string;
  placeholder?: string;
};
type MapField = NamedField & {
  kind: "map";
  keyLabel: string;
  valueLabel: string;
  valueKind?: "text" | "password" | "number" | "list";
  min?: number;
  max?: number;
};
export type Field =
  | TextField
  | (NamedField & {
      kind: "number";
      min?: number;
      max?: number;
      optional?: boolean;
    })
  | (NamedField & { kind: "select"; choices: Choices; optional?: boolean })
  | (NamedField & { kind: "boolean" | "list" })
  | (NamedField & { kind: "choices"; choices: Choices })
  | (NamedField & { kind: "section"; fields: readonly Field[] })
  | (NamedField & {
      kind: "object";
      fields: readonly Field[];
      create?: () => object;
      collapsible?: boolean;
    })
  | (NamedField & {
      kind: "rows";
      fields: readonly Field[];
      create: () => object;
      ordered?: boolean;
      max?: number;
    })
  | MapField;

const idPattern = "[A-Za-z0-9_\\-]+";
export const identifier = (
  label: string,
  readOnly = false,
  maxLength = 64,
): Field => ({
  kind: "text",
  key: "id",
  label,
  required: true,
  pattern: idPattern,
  readOnly,
  maxLength,
});

export const number = (
  key: string,
  label: string,
  min: number,
  max: number,
  optional = false,
): Field => ({
  kind: "number",
  key,
  label,
  min,
  max,
  optional,
});

export const text = (key: string, label: string, required = false): Field => ({
  kind: "text",
  key,
  label,
  required,
});

export const optionalText = (key: string, label: string): Field => ({
  kind: "text",
  key,
  label,
  nullable: true,
});
export const toggle = (key: string, label: string): Field => ({
  kind: "boolean",
  key,
  label,
});
export const list = (key: string, label: string): Field => ({
  kind: "list",
  key,
  label,
});
export const select = (
  key: string,
  label: string,
  choices: Choices,
  optional = false,
): Field => ({
  kind: "select",
  key,
  label,
  choices,
  optional,
});
export const protocols = { http: "HTTP", https: "HTTPS", socks5: "SOCKS5" };
export const transports = { tcp: "TCP", udp: "UDP" };

export function ConfigFields<T extends object>({
  fields,
  value,
  onChange,
}: {
  fields: readonly Field[];
  value: T;
  onChange: (value: T) => void;
}) {
  const record = value as RecordValue;
  return (
    <div className="grid min-w-0 items-start gap-3 sm:grid-cols-2">
      {fields.map(
        (field) =>
          (!field.when || field.when(record)) && (
            <FieldInput
              key={field.key}
              field={field}
              record={record}
              update={(next) =>
                onChange(
                  field.kind === "section"
                    ? (next as T)
                    : { ...value, [field.key]: next },
                )
              }
            />
          ),
      )}
    </div>
  );
}

function FieldInput({
  field,
  record,
  update,
}: {
  field: Field;
  record: RecordValue;
  update: (value: unknown) => void;
}) {
  const description = useId();
  const value = record[field.key];
  const help = field.help && (
    <span id={description} className="muted text-xs leading-5 font-normal">
      {field.help}
    </span>
  );
  const describedBy = field.help ? description : undefined;
  const block =
    "min-w-0 rounded-lg border border-console-border p-3 sm:col-span-2";
  switch (field.kind) {
    case "section":
      return (
        <details className={block} data-config-key={field.key}>
          <summary className="cursor-pointer text-sm font-medium">
            {field.label}
          </summary>
          <div className="mt-3 space-y-2">
            {help}
            <ConfigFields
              fields={field.fields}
              value={record}
              onChange={update}
            />
          </div>
        </details>
      );
    case "object": {
      const content = (
        <>
          {help}
          {value && typeof value === "object" && (
            <ConfigFields
              fields={field.fields}
              value={value}
              onChange={update}
            />
          )}
        </>
      );
      if (field.create) {
        const create = field.create;
        return (
          <fieldset className={block}>
            <legend className="px-1 text-sm font-medium">{field.label}</legend>
            <label className="mb-2 flex items-center gap-2 text-xs">
              <input
                type="checkbox"
                checked={Boolean(value)}
                onChange={(e) => update(e.target.checked ? create() : null)}
              />
              配置{field.label}
            </label>
            {content}
          </fieldset>
        );
      }
      if (field.collapsible) {
        return (
          <details className={block} data-config-key={field.key}>
            <summary className="cursor-pointer text-sm font-medium">
              {field.label}
            </summary>
            <div className="mt-3 space-y-2">{content}</div>
          </details>
        );
      }
      return (
        <fieldset className={block}>
          <legend className="px-1 text-sm font-medium">{field.label}</legend>
          {content}
        </fieldset>
      );
    }
    case "rows":
      return (
        <Rows
          field={field}
          rows={Array.isArray(value) ? value : []}
          update={update}
        />
      );
    case "map":
      return (
        <MapRows
          field={field}
          initial={value as RecordValue | undefined}
          update={update}
        />
      );
    case "boolean":
      return (
        <label className="flex min-h-8 items-center gap-2 text-sm">
          <input
            type="checkbox"
            checked={value === true}
            onChange={(e) => update(e.target.checked)}
            aria-describedby={describedBy}
          />
          {field.label}
          {help}
        </label>
      );
    case "choices": {
      const chosen = Array.isArray(value) ? value : [];
      return (
        <fieldset className="min-w-0">
          <legend className="text-xs font-medium">{field.label}</legend>
          <div className="mt-1 flex flex-wrap gap-x-3 gap-y-1">
            {Object.entries(field.choices).map(([option, label]) => (
              <label key={option} className="flex items-center gap-1.5 text-sm">
                <input
                  type="checkbox"
                  checked={chosen.includes(option)}
                  onChange={(e) =>
                    update(
                      e.target.checked
                        ? [...chosen, option]
                        : chosen.filter((item) => item !== option),
                    )
                  }
                />
                {label}
              </label>
            ))}
          </div>
          {help}
        </fieldset>
      );
    }
    case "list":
      return (
        <label className="form-field">
          {field.label}
          <LineInput value={value} update={update} describedBy={describedBy} />
          {help}
        </label>
      );
    case "select":
      return (
        <label className="form-field">
          {field.label}
          <select
            className="field"
            value={String(value ?? "")}
            onChange={(e) => update(e.target.value || null)}
            required={!field.optional}
            aria-describedby={describedBy}
          >
            <option value="" disabled={!field.optional}>
              {field.optional ? "未设置" : "请选择"}
            </option>
            {Object.entries(field.choices).map(([option, label]) => (
              <option key={option} value={option}>
                {label}
              </option>
            ))}
            {typeof value === "string" &&
              value &&
              !Object.prototype.hasOwnProperty.call(field.choices, value) && (
                <option value={value}>{value}</option>
              )}
          </select>
          {help}
        </label>
      );
    case "number":
      return (
        <label className="form-field">
          {field.label}
          <input
            className="field"
            type="number"
            step={1}
            min={field.min}
            max={field.max}
            required={!field.optional}
            value={typeof value === "number" ? value : ""}
            onChange={(e) =>
              update(e.target.value === "" ? null : e.target.valueAsNumber)
            }
            aria-describedby={describedBy}
          />
          {help}
        </label>
      );
    default:
      return (
        <label className="form-field">
          {field.label}
          <input
            className="field"
            type={field.kind}
            required={field.required}
            readOnly={field.readOnly}
            maxLength={field.maxLength}
            pattern={field.pattern}
            placeholder={field.placeholder}
            value={String(value ?? "")}
            onChange={(e) =>
              update(e.target.value || (field.nullable ? null : ""))
            }
            autoComplete={field.kind === "password" ? "new-password" : "off"}
            spellCheck={false}
            aria-describedby={describedBy}
          />
          {help}
        </label>
      );
  }
}

function Rows({
  field,
  rows,
  update,
}: {
  field: Extract<Field, { kind: "rows" }>;
  rows: object[];
  update: (value: object[]) => void;
}) {
  function move(index: number, direction: number) {
    const next = [...rows];
    const other = index + direction;
    [next[index], next[other]] = [next[other], next[index]];
    update(next);
  }
  return (
    <fieldset className="min-w-0 space-y-2 rounded-lg border border-console-border p-3 sm:col-span-2">
      <legend className="px-1 text-sm font-medium">{field.label}</legend>
      {field.help && <p className="muted text-xs">{field.help}</p>}
      {rows.map((row, index) => (
        <fieldset
          className="min-w-0 space-y-2 rounded-md bg-console-soft p-3"
          key={index}
        >
          <legend className="px-1 text-xs font-medium">
            {field.label} {index + 1}
          </legend>
          <ConfigFields
            fields={field.fields}
            value={row}
            onChange={(next) =>
              update(rows.map((item, i) => (i === index ? next : item)))
            }
          />
          <div className="flex flex-wrap gap-2">
            {field.ordered && (
              <>
                <button
                  type="button"
                  className="btn text-xs"
                  disabled={index === 0}
                  onClick={() => move(index, -1)}
                >
                  上移
                </button>
                <button
                  type="button"
                  className="btn text-xs"
                  disabled={index === rows.length - 1}
                  onClick={() => move(index, 1)}
                >
                  下移
                </button>
              </>
            )}
            <button
              type="button"
              className="btn text-xs"
              onClick={() => update(rows.filter((_, i) => i !== index))}
            >
              <Minus size={14} />
              移除
            </button>
          </div>
        </fieldset>
      ))}
      <button
        type="button"
        className="btn"
        disabled={field.max !== undefined && rows.length >= field.max}
        onClick={() => update([...rows, field.create()])}
      >
        <Plus size={14} />
        添加{field.label}
      </button>
    </fieldset>
  );
}

function MapRows({
  field,
  initial,
  update,
}: {
  field: MapField;
  initial?: RecordValue;
  update: (value: RecordValue) => void;
}) {
  type Row = [string, unknown];
  const [draft, setDraft] = useState<{ record: RecordValue; rows: Row[] }>();
  const rows =
    draft && draft.record === initial
      ? draft.rows
      : Object.entries(initial ?? {});
  const ids = useId();
  const seen = new Set<string>();
  const duplicates = new Set<string>();
  for (const [key] of rows) {
    if (seen.has(key)) duplicates.add(key);
    seen.add(key);
  }
  function change(next: Row[]) {
    const record = Object.fromEntries(next);
    setDraft({ record, rows: next });
    update(record);
  }
  return (
    <fieldset className="min-w-0 space-y-2 rounded-lg border border-console-border p-3 sm:col-span-2">
      <legend className="px-1 text-sm font-medium">{field.label}</legend>
      {field.help && <p className="muted text-xs">{field.help}</p>}
      {rows.map(([key, value], index) => {
        const duplicate = duplicates.has(key);
        const errorId = ids + index;
        const replace = (row: Row) =>
          change(rows.map((item, i) => (i === index ? row : item)));
        return (
          <div
            className="grid items-start gap-2 sm:grid-cols-[minmax(0,1fr)_minmax(0,1fr)_auto]"
            key={index}
          >
            <label className="form-field">
              {field.keyLabel}
              <input
                className="field"
                required
                value={key}
                onChange={(e) => replace([e.target.value, value])}
                spellCheck={false}
                aria-invalid={duplicate}
                aria-describedby={duplicate ? errorId : undefined}
                ref={(input) =>
                  input?.setCustomValidity(duplicate ? "名称已存在" : "")
                }
              />
              {duplicate && (
                <span id={errorId} className="text-console-danger">
                  名称已存在
                </span>
              )}
            </label>
            {field.valueKind === "list" ? (
              <label className="form-field">
                {field.valueLabel}
                <LineInput
                  value={value}
                  update={(next) => replace([key, next])}
                />
              </label>
            ) : (
              <label className="form-field">
                {field.valueLabel}
                <input
                  className="field"
                  type={field.valueKind ?? "text"}
                  autoComplete={
                    field.valueKind === "password" ? "new-password" : "off"
                  }
                  value={String(value ?? "")}
                  required={field.valueKind === "number"}
                  min={field.min}
                  max={field.max}
                  step={field.valueKind === "number" ? 1 : undefined}
                  onChange={(e) =>
                    replace([
                      key,
                      field.valueKind === "number"
                        ? e.target.value
                          ? e.target.valueAsNumber
                          : null
                        : e.target.value,
                    ])
                  }
                />
              </label>
            )}
            <button
              type="button"
              className="btn text-xs sm:mt-5"
              onClick={() => change(rows.filter((_, i) => i !== index))}
              aria-label={`移除${field.label} ${index + 1}`}
            >
              <Minus size={14} />
              移除
            </button>
          </div>
        );
      })}
      <button
        type="button"
        className="btn"
        onClick={() =>
          change([...rows, ["", field.valueKind === "list" ? [] : ""]])
        }
      >
        <Plus size={14} />
        添加{field.label}
      </button>
    </fieldset>
  );
}

function LineInput({
  value,
  update,
  describedBy,
}: {
  value: unknown;
  update: (value: string[]) => void;
  describedBy?: string;
}) {
  const [draft, setDraft] = useState<{ items: string[]; text: string }>();
  const contents =
    draft && draft.items === value
      ? draft.text
      : Array.isArray(value)
        ? value.join("\n")
        : "";
  return (
    <textarea
      className="field"
      rows={2}
      value={contents}
      onChange={(event) => {
        const text = event.target.value;
        const items = text
          .split("\n")
          .map((item) => item.trim())
          .filter(Boolean);
        setDraft({ items, text });
        update(items);
      }}
      placeholder="每行一项"
      spellCheck={false}
      aria-describedby={describedBy}
    />
  );
}

export function ConfigForm({
  label,
  className = "space-y-3",
  children,
  onSave,
}: {
  label: string;
  className?: string;
  children: ReactNode;
  onSave: () => void;
}) {
  return (
    <form
      aria-label={label}
      className={className}
      onSubmit={(event) => {
        event.preventDefault();
        onSave();
      }}
      onInvalidCapture={(event) => {
        if (!(event.target instanceof HTMLElement)) return;
        // WebKit can skip a control that was hidden when validation began.
        const form = event.currentTarget;
        let revealed = false;
        for (
          let parent = event.target.parentElement;
          parent && parent !== form;
          parent = parent.parentElement
        ) {
          if (parent instanceof HTMLDetailsElement && !parent.open) {
            parent.open = true;
            revealed = true;
          }
        }
        if (revealed)
          requestAnimationFrame(() => {
            form
              .querySelector<HTMLElement>(
                "input:invalid, select:invalid, textarea:invalid",
              )
              ?.focus();
          });
      }}
    >
      {children}
    </form>
  );
}
