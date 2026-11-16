import { useEffect, useId, useState, type Ref } from "react";
import { ArrowDown, ArrowUp, X } from "lucide-react";
import { getJson } from "../platform";
import { nodeStates, type NodesOverview } from "../nodes";
import { subscriptionApi } from "../subscriptions";
import type { ProviderView } from "../types";

export type Choices = Readonly<Record<string, string>>;
export type Catalog = {
  choices: Choices;
  notice?: string;
  search?: { group?: string };
};
const empty: Catalog = { choices: {} };
const labelFor = (choices: Choices, value: string) =>
  Object.prototype.hasOwnProperty.call(choices, value)
    ? choices[value]
    : undefined;

export function idChoices(ids: readonly string[]): Catalog {
  return { choices: Object.fromEntries(ids.map((id) => [id, id])) };
}

async function readNodes(
  search: string,
  group: string | undefined,
  signal: AbortSignal,
): Promise<Catalog> {
  const query = new URLSearchParams({
    limit: "50",
    hide_expired: "true",
    hide_deleted: "true",
    hide_retired: "true",
  });
  if (group) {
    query.set("group", group);
    query.set("ignore_group_pin", "true");
  }
  if (search) query.set("search", search);
  const data = await getJson<NodesOverview>(`api/v1/nodes?${query}`, signal);
  return {
    choices: Object.fromEntries(
      data.nodes.map((node) => [
        node.id,
        `${node.id} · ${node.protocol.toUpperCase()} ${node.host.includes(":") ? `[${node.host}]` : node.host}:${node.port}${!node.configuration_enabled ? " · 已停用" : node.operation.state !== "enabled" ? ` · ${nodeStates[node.operation.state]}` : ""}`,
      ]),
    ),
    notice:
      data.next_offset !== null || data.inventory_truncated
        ? "仅显示前 50 个匹配项，可搜索 ID、地址或地区。"
        : undefined,
    search:
      data.next_offset !== null || data.inventory_truncated
        ? { group }
        : undefined,
  };
}

async function readCatalog(
  kind: "inbounds" | "sources" | "nodes",
  group: string | undefined,
  signal: AbortSignal,
): Promise<Catalog> {
  if (kind === "nodes") return readNodes("", group, signal);
  if (kind === "inbounds") {
    const data = await getJson<{
      inbounds: { id: string; protocol: string }[];
    }>("api/v1/inbounds", signal);
    return {
      choices: Object.fromEntries(
        data.inbounds.map((item) => [
          item.id,
          `${item.id} · ${item.protocol.toUpperCase()}`,
        ]),
      ),
    };
  }
  const [providers, subscriptions] = await Promise.all([
    getJson<ProviderView[]>("api/v1/providers", signal),
    subscriptionApi.list(signal),
  ]);
  const sources = [
    ...providers.map(({ configuration: { id, name } }) => ({ id, name })),
    ...subscriptions.subscriptions.map(({ id, name }) => ({
      id: `subscription:${id}`,
      name,
    })),
  ];
  return {
    choices: Object.fromEntries(
      sources.map(({ id, name }) => [
        id,
        name && name !== id ? `${name} · ${id}` : id,
      ]),
    ),
  };
}

export function useReferenceCatalog(
  kind: "inbounds" | "sources" | "nodes",
  revision: number | undefined,
  group?: string,
): Catalog {
  const key =
    revision === undefined
      ? undefined
      : JSON.stringify([kind, revision, group]);
  const [result, setResult] = useState<{ key: string; catalog: Catalog }>();
  useEffect(() => {
    if (key === undefined) return;
    const controller = new AbortController();
    void readCatalog(kind, group, controller.signal).then(
      (catalog) => {
        if (!controller.signal.aborted) setResult({ key, catalog });
      },
      () => {
        if (!controller.signal.aborted)
          setResult({
            key,
            catalog: { ...empty, notice: "列表读取失败，请刷新页面后重试。" },
          });
      },
    );
    return () => controller.abort();
  }, [key, kind, group]);
  return result && result.key === key
    ? result.catalog
    : { ...empty, notice: "正在读取可选项…" };
}

export function ReferenceSelect({
  label,
  value,
  catalog,
  onChange,
  required = false,
  placeholder,
  disabled,
  exclude,
  invalid,
  inputRef,
}: {
  label: string;
  value: string;
  catalog: Catalog;
  onChange: (value: string, label?: string) => void;
  required?: boolean;
  placeholder?: string;
  disabled?: boolean;
  exclude?: ReadonlySet<string>;
  invalid?: boolean;
  inputRef?: Ref<HTMLSelectElement>;
}) {
  const id = useId();
  const [search, setSearch] = useState("");
  const [result, setResult] = useState<{
    query: string;
    group?: string;
    catalog: Catalog;
  }>();
  const searchable = Boolean(catalog.search);
  const group = catalog.search?.group;
  const query = searchable ? search.trim() : "";
  useEffect(() => {
    if (!query) return;
    const controller = new AbortController();
    const timer = window.setTimeout(() => {
      void readNodes(query, group, controller.signal).then(
        (matches) => {
          if (!controller.signal.aborted)
            setResult({ query, group, catalog: matches });
        },
        () => {
          if (!controller.signal.aborted)
            setResult({
              query,
              group,
              catalog: { ...empty, notice: "搜索失败，请修改关键词重试。" },
            });
        },
      );
    }, 250);
    return () => {
      window.clearTimeout(timer);
      controller.abort();
    };
  }, [query, group]);
  const visible = !query
    ? catalog
    : result?.query === query && result.group === group
      ? result.catalog
      : { ...empty, notice: "正在搜索节点…" };
  const options = Object.entries(visible.choices).filter(
    ([key]) => !exclude?.has(key) || key === value,
  );
  const saved = value && labelFor(visible.choices, value) === undefined;
  const notice =
    visible.notice ??
    (!options.length ? (query ? "未找到匹配项。" : "暂无可选项。") : undefined);
  return (
    <div className="min-w-0 space-y-1">
      {searchable && (
        <label className="form-field">
          搜索{label}
          <input
            className="field"
            type="search"
            maxLength={80}
            placeholder="输入节点 ID、地址或地区"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") event.preventDefault();
            }}
            disabled={disabled}
          />
        </label>
      )}
      <label className="form-field">
        {label}
        <select
          className="field"
          value={value}
          required={required}
          disabled={disabled}
          aria-invalid={invalid || undefined}
          aria-describedby={notice ? id : undefined}
          ref={inputRef}
          onChange={(event) =>
            onChange(
              event.target.value,
              labelFor(visible.choices, event.target.value),
            )
          }
        >
          <option value="" disabled={required}>
            {placeholder ?? (required ? "请选择" : "未设置")}
          </option>
          {saved && (
            <option value={value}>
              {labelFor(catalog.choices, value) ?? `${value}（当前列表未列出）`}
            </option>
          )}
          {options.map(([key, label]) => (
            <option key={key} value={key}>
              {label}
            </option>
          ))}
        </select>
      </label>
      {notice && (
        <p
          id={id}
          className="muted text-xs"
          aria-live={query ? "polite" : undefined}
        >
          {notice}
        </p>
      )}
    </div>
  );
}

export function ReferenceList({
  label,
  values,
  catalog,
  onChange,
  ordered,
  disabled,
  exclude = [],
}: {
  label: string;
  values: string[];
  catalog: Catalog;
  onChange: (values: string[]) => void;
  ordered?: boolean;
  disabled?: boolean;
  exclude?: readonly string[];
}) {
  const [labels, setLabels] = useState<Choices>({});
  function move(index: number, offset: number) {
    const next = [...values];
    [next[index], next[index + offset]] = [next[index + offset], next[index]];
    onChange(next);
  }
  return (
    <div className="min-w-0 space-y-2">
      <ReferenceSelect
        label={label}
        catalog={catalog}
        value=""
        placeholder="请选择要添加的项"
        exclude={new Set([...values, ...exclude])}
        disabled={disabled}
        onChange={(id, name) => {
          if (!id) return;
          if (name) setLabels((current) => ({ ...current, [id]: name }));
          onChange([...values, id]);
        }}
      />
      {values.length > 0 && (
        <ul aria-label={`已选${label}`} className="space-y-1">
          {values.map((id, index) => (
            <li
              key={id}
              className="flex min-w-0 items-center gap-1 rounded-lg bg-console-soft px-2"
            >
              <span className="min-w-0 flex-1 break-all text-xs">
                {labelFor(catalog.choices, id) ?? labelFor(labels, id) ?? id}
              </span>
              {ordered && (
                <>
                  <button
                    type="button"
                    className="btn w-8 shrink-0 p-0"
                    aria-label={`上移 ${id}`}
                    disabled={disabled || index === 0}
                    onClick={() => move(index, -1)}
                  >
                    <ArrowUp size={14} />
                  </button>
                  <button
                    type="button"
                    className="btn w-8 shrink-0 p-0"
                    aria-label={`下移 ${id}`}
                    disabled={disabled || index === values.length - 1}
                    onClick={() => move(index, 1)}
                  >
                    <ArrowDown size={14} />
                  </button>
                </>
              )}
              <button
                type="button"
                className="btn w-8 shrink-0 p-0"
                aria-label={`移除 ${id}`}
                disabled={disabled}
                onClick={() => onChange(values.filter((item) => item !== id))}
              >
                <X size={14} />
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
