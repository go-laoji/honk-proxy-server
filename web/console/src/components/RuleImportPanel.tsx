import { useEffect, useRef, useState } from "react";
import { getJson, postJson, saveRules } from "../platform";
type Preview = {
  format_version: number;
  checksum: string;
  rule_count: number;
  validation: {
    valid: boolean;
    diagnostics: { path: string; message: string }[];
    changes: { path: string }[];
  };
};
export default function RuleImportPanel({
  revision,
  viewed,
  onImport,
  locked = false,
}: {
  revision: number;
  viewed: number;
  locked?: boolean;
  onImport: (document: unknown, checksum: string) => Promise<void>;
}) {
  const [body, setBody] = useState(""),
    [preview, setPreview] = useState<{ value: Preview; document: unknown }>(),
    [busy, setBusy] = useState(false),
    [error, setError] = useState(""),
    [info, setInfo] = useState("");
  const disabled = busy || locked;
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  function edit(value: string) {
    setBody(value);
    setPreview(undefined);
    setInfo("");
    setError("");
  }
  async function run(work: () => Promise<void>) {
    setBusy(true);
    setError("");
    setInfo("");
    try {
      await work();
    } catch (cause) {
      if (mounted.current)
        setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      if (mounted.current) setBusy(false);
    }
  }
  return (
    <section
      className="panel panel-body space-y-4"
      aria-label="规则批量导入导出"
    >
      <h2 className="font-semibold">规则批量导入导出</h2>
      <p className="muted text-xs leading-6">
        JSON 格式版本 1，仅包含规则与引用，不导出上游凭据。预览最多 1
        MiB；创建候选后仍需显式激活。来源版本只作说明，引用按当前服务重新检查。
      </p>
      <div className="flex flex-wrap gap-2">
        <button
          className="btn"
          disabled={disabled}
          onClick={() =>
            void run(async () => {
              const value = await getJson<unknown>(
                `api/v1/rules/export?revision=${viewed}`,
              );
              if (mounted.current) edit(JSON.stringify(value, null, 2));
            })
          }
        >
          读取所看规则到编辑框
        </button>
        <button
          className="btn"
          disabled={disabled}
          onClick={() =>
            void run(async () => {
              const saved = await saveRules(viewed);
              if (mounted.current)
                setInfo(
                  saved ? `规则 r${viewed} 文件已导出` : "已取消文件保存",
                );
            })
          }
        >
          导出规则文件
        </button>
        <label className="btn cursor-pointer">
          选择规则 JSON
          <input
            className="sr-only"
            type="file"
            accept=".json,application/json"
            disabled={disabled}
            onChange={(event) => {
              const file = event.target.files?.[0];
              if (!file) return;
              void run(async () => {
                if (file.size > 1024 * 1024)
                  throw new Error("规则 JSON 超过 1 MiB");
                const text = await file.text();
                if (mounted.current) edit(text);
              });
              event.target.value = "";
            }}
          />
        </label>
      </div>
      <label className="form-field">
        规则导入 JSON
        <textarea
          aria-label="规则导入 JSON"
          className="field editor-field min-h-48"
          value={body}
          disabled={disabled}
          onChange={(event) => edit(event.target.value)}
          spellCheck={false}
        />
      </label>
      <div className="flex flex-wrap gap-2">
        <button
          className="btn"
          disabled={disabled || !body.trim()}
          onClick={() =>
            void run(async () => {
              if (new TextEncoder().encode(body).byteLength > 1024 * 1024)
                throw new Error("规则 JSON 超过 1 MiB");
              let document: unknown;
              try {
                document = JSON.parse(body);
              } catch {
                throw new Error("JSON 语法无效，请检查格式版本和字段");
              }
              const value = await postJson<Preview>(
                "api/v1/rules/import/preview",
                { document, expected_revision: revision },
              );
              if (mounted.current) setPreview({ value, document });
            })
          }
        >
          预览规则导入
        </button>
        <button
          className="btn-primary"
          disabled={disabled || !preview?.value.validation.valid}
          onClick={() =>
            preview &&
            void run(async () => {
              try {
                await onImport(preview.document, preview.value.checksum);
                if (mounted.current) {
                  setPreview(undefined);
                  setInfo(
                    "规则候选已保存；尚未激活，请在版本区核查后显式激活。",
                  );
                }
              } catch (cause) {
                throw new Error(
                  `${cause instanceof Error ? cause.message : String(cause)}；可能已创建候选，请先刷新版本列表确认。`,
                );
              }
            })
          }
        >
          创建导入候选
        </button>
      </div>
      {preview && (
        <div className="rounded-lg bg-console-soft p-4 text-xs">
          <p>
            {preview.value.validation.valid ? "预览有效" : "预览未通过"} ·{" "}
            {preview.value.rule_count} 条规则 ·{" "}
            {preview.value.validation.changes.length} 项变更 · 基于 r{revision}
          </p>
          <ul className="mt-2 space-y-1">
            {preview.value.validation.diagnostics.map((item, index) => (
              <li key={index}>
                {item.path}: {item.message}
              </li>
            ))}
          </ul>
          <p className="muted mt-2 break-all">
            校验和 {preview.value.checksum}
          </p>
        </div>
      )}
      {error && (
        <p role="alert" className="break-all text-sm text-red-600">
          {error}
        </p>
      )}
      {info && (
        <p role="status" className="text-sm text-console-accent-ink">
          {info}
        </p>
      )}
    </section>
  );
}
