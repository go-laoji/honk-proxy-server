import { useEffect, useId, useState, type FormEvent } from "react";
import { Layers3, Plus } from "lucide-react";
import { getJson, postJson } from "../platform";
import {
  blankGroup,
  type GroupsOverview,
  type Policy,
  type ProviderRegionRule,
} from "../selection";
import type { LeaseInfo } from "../types";

const fields = [
  ["isp", "运营商"],
  ["province", "省份"],
  ["city", "城市"],
] as const;
const newGroup = "@new";
const groupIdPattern = "[A-Za-z0-9_\\-]{1,64}";
const message = (cause: unknown) =>
  cause instanceof Error ? cause.message : "操作失败";

type RuleAddress = { group: string; index: number };
type Draft = {
  group: string;
  groupId: string;
  isp: string;
  province: string;
  city: string;
  original?: RuleAddress;
};
const emptyDraft = { group: "", groupId: "", isp: "", province: "", city: "" };

export default function ProviderRegionGroups({
  provider,
  leases,
  onSaved,
}: {
  provider: string;
  leases: LeaseInfo[];
  onSaved: (previous: number, revision: number) => Promise<void>;
}) {
  const [data, setData] = useState<GroupsOverview>();
  const [draft, setDraft] = useState<Draft>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [reload, setReload] = useState(0);
  const listId = useId();
  useEffect(() => {
    const controller = new AbortController();
    void getJson<GroupsOverview>("api/v1/groups", controller.signal)
      .then((result) => {
        if (!controller.signal.aborted) setData(result);
      })
      .catch((cause: unknown) => {
        if (!controller.signal.aborted) setError(message(cause));
      });
    return () => controller.abort();
  }, [reload]);

  const groups = data?.configuration.groups ?? [];
  const rules = groups.flatMap((group) =>
    (group.provider_regions ?? []).flatMap((rule, index) =>
      rule.provider === provider ? [{ group: group.id, index, rule }] : [],
    ),
  );
  const locations = leases.flatMap((lease) =>
    lease.location ? [lease.location] : [],
  );
  const samples = [
    ...new Map(
      locations.map((location) => [
        [location.province, location.city, location.isp]
          .filter(Boolean)
          .join(" · "),
        location,
      ]),
    ).entries(),
  ].filter(([label]) => label);
  const target = groups.find((group) => group.id === draft?.group);

  function configure(rule?: RuleAddress & { rule: ProviderRegionRule }) {
    setError("");
    setNotice("");
    setDraft(
      rule
        ? {
            group: rule.group,
            groupId: "",
            original: { group: rule.group, index: rule.index },
            isp: rule.rule.isp ?? "",
            province: rule.rule.province ?? "",
            city: rule.rule.city ?? "",
          }
        : { ...emptyDraft, group: groups.length ? "" : newGroup },
    );
  }

  async function apply(configuration: Policy, success: string) {
    if (!data) return;
    setBusy(true);
    setError("");
    setNotice("");
    try {
      const result = await postJson<{
        state: string;
        running_revision: number;
      }>("api/v1/groups/save", {
        configuration,
        expected_revision: data.running_revision,
      });
      setDraft(undefined);
      setNotice(
        result.state === "applied" ? success : "规则已保存，等待服务重启。",
      );
      setData(undefined);
      setReload((value) => value + 1);
      await onSaved(data.running_revision, result.running_revision);
    } catch (cause) {
      setError(message(cause));
    } finally {
      setBusy(false);
    }
  }

  function withoutRule(address?: RuleAddress) {
    return groups.map((group) =>
      group.id === address?.group
        ? {
            ...group,
            provider_regions: group.provider_regions?.filter(
              (_, index) => index !== address.index,
            ),
          }
        : group,
    );
  }

  function save(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!data || !draft || busy) return;
    const rule = {
      provider,
      isp: draft.isp.trim() || null,
      province: draft.province.trim() || null,
      city: draft.city.trim() || null,
    };
    if (!rule.isp && !rule.province && !rule.city) {
      setError("请填写至少一项地区条件。");
      return;
    }
    const next = withoutRule(draft.original);
    const groupId =
      draft.group === newGroup ? draft.groupId.trim() : draft.group;
    if (draft.group === newGroup) {
      if (groups.some((group) => group.id === groupId)) {
        setError("该分组已存在，请从目标组列表选择。");
        return;
      }
      next.push({ ...blankGroup, id: groupId, provider_regions: [rule] });
    } else {
      const group = next.find((group) => group.id === groupId);
      if (!group) return;
      if (
        group.provider_regions?.some(
          (entry) =>
            fields.every(([key]) => entry[key] === rule[key]) &&
            entry.provider === provider,
        )
      ) {
        setError("此分组已有相同规则。");
        return;
      }
      next[next.indexOf(group)] = {
        ...group,
        provider_regions: [...(group.provider_regions ?? []), rule],
      };
    }
    void apply(
      { ...data.configuration, groups: next },
      `规则已应用到 ${groupId}。`,
    );
  }

  return (
    <section
      id="provider-region-groups"
      className="panel panel-body space-y-3"
      aria-label="按地区加入代理组"
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h2 className="flex items-center gap-2 font-semibold">
          <Layers3 size={16} aria-hidden="true" />
          按地区加入代理组
        </h2>
        <button
          type="button"
          className="btn"
          disabled={!data || busy || !!draft}
          onClick={() => configure()}
        >
          <Plus size={14} aria-hidden="true" />
          新增规则
        </button>
      </div>
      {notice && (
        <p role="status" className="text-sm text-console-success">
          {notice}
        </p>
      )}
      {error && (
        <p role="alert" className="text-sm text-console-danger">
          {error}
        </p>
      )}
      {error && (
        <button
          className="btn"
          type="button"
          onClick={() => {
            setError("");
            setDraft(undefined);
            setData(undefined);
            setReload((value) => value + 1);
          }}
          disabled={busy}
        >
          重新读取分组
        </button>
      )}
      {!data && !error && <p className="muted text-sm">正在读取分组…</p>}
      {data && rules.length === 0 && !draft && (
        <p className="muted text-sm">暂无地区入组规则。</p>
      )}
      {rules.length > 0 && (
        <ul
          className="divide-y divide-console-border"
          aria-label="地区入组规则"
        >
          {rules.map((entry) => (
            <li
              key={`${entry.group}:${entry.index}`}
              className="flex flex-wrap items-center justify-between gap-2 py-2 text-sm"
            >
              <div className="flex flex-wrap items-center gap-2">
                <span>
                  {fields
                    .flatMap(([key]) =>
                      entry.rule[key] ? [entry.rule[key]] : [],
                    )
                    .join(" · ")}
                </span>
                <a
                  className="text-console-accent-ink"
                  href={`./nodes.html?${new URLSearchParams({ provider, group: entry.group })}`}
                >
                  → {entry.group}
                </a>
              </div>
              <div className="flex gap-2">
                <button
                  type="button"
                  className="btn"
                  disabled={busy || !!draft}
                  onClick={() => configure(entry)}
                >
                  编辑规则
                </button>
                <button
                  type="button"
                  className="btn"
                  disabled={busy || !!draft}
                  onClick={() =>
                    data &&
                    void apply(
                      { ...data.configuration, groups: withoutRule(entry) },
                      "已移除地区规则。",
                    )
                  }
                >
                  移除规则
                </button>
              </div>
            </li>
          ))}
        </ul>
      )}
      {draft && (
        <form
          onSubmit={save}
          aria-label="地区规则编辑"
          className="space-y-3 border-t border-console-border pt-3"
        >
          {samples.length > 0 && (
            <label className="form-field">
              从租约填入地区
              <select
                className="field"
                value=""
                disabled={busy}
                onChange={(event) => {
                  if (event.target.value === "") return;
                  const location = samples[Number(event.target.value)]?.[1];
                  if (location)
                    setDraft({
                      ...draft,
                      isp: location.isp ?? "",
                      province: location.province ?? "",
                      city: location.city ?? "",
                    });
                }}
              >
                <option value="">选择一条已识别的地区</option>
                {samples.map(([label], index) => (
                  <option key={label} value={index}>
                    {label}
                  </option>
                ))}
              </select>
            </label>
          )}
          <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
            {fields.map(([key, label]) => (
              <label className="form-field" key={key}>
                {label}
                <input
                  className="field"
                  value={draft[key]}
                  list={`${listId}-${key}`}
                  placeholder="不限"
                  maxLength={96}
                  disabled={busy}
                  onChange={(event) =>
                    setDraft({ ...draft, [key]: event.target.value })
                  }
                />
                <datalist id={`${listId}-${key}`}>
                  {[
                    ...new Set(
                      locations.flatMap((location) =>
                        location[key] ? [location[key]] : [],
                      ),
                    ),
                  ]
                    .sort()
                    .map((value) => (
                      <option key={value} value={value} />
                    ))}
                </datalist>
              </label>
            ))}
            <label className="form-field">
              目标代理组
              <select
                className="field"
                value={draft.group}
                disabled={busy}
                required
                onChange={(event) =>
                  setDraft({ ...draft, group: event.target.value })
                }
              >
                <option value="">选择代理组</option>
                {groups.map((group) => (
                  <option key={group.id} value={group.id}>
                    {group.id}
                  </option>
                ))}
                <option value={newGroup}>新建代理组…</option>
              </select>
            </label>
          </div>
          {draft.group === newGroup && (
            <label className="form-field max-w-sm">
              新组 ID
              <input
                className="field"
                required
                pattern={groupIdPattern}
                maxLength={64}
                placeholder="例如 guangdong-telecom"
                value={draft.groupId}
                disabled={busy}
                onChange={(event) =>
                  setDraft({ ...draft, groupId: event.target.value })
                }
              />
            </label>
          )}
          <p className="muted text-xs">
            使用端点 IP 匹配。填写的条件需同时满足，留空的条件不限。
          </p>
          {(target?.include_all || target?.providers.includes(provider)) && (
            <p className="muted text-xs">
              此组已有全量成员规则，同一服务商的其他节点仍可入组。
            </p>
          )}
          <div className="flex flex-wrap gap-2">
            <button
              type="submit"
              className="btn-primary"
              disabled={busy || !data}
            >
              {busy ? "正在保存…" : "保存地区规则"}
            </button>
            <button
              type="button"
              className="btn"
              disabled={busy}
              onClick={() => setDraft(undefined)}
            >
              取消
            </button>
          </div>
        </form>
      )}
    </section>
  );
}
