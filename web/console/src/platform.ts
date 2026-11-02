export const isDesktop = "__TAURI_INTERNALS__" in window;

async function native<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(command, args);
}

export async function endpoint(): Promise<string> {
  return isDesktop
    ? native<string>("get_endpoint")
    : new URL("../", window.location.href).href;
}

export async function connect(value: string): Promise<void> {
  const url = new URL(value);
  if (
    !["http:", "https:"].includes(url.protocol) ||
    url.username ||
    url.password ||
    url.search ||
    url.hash
  ) {
    throw new Error("请输入不含账号、查询参数的 HTTP(S) 服务地址");
  }
  if (isDesktop) {
    await native("set_endpoint", { endpoint: url.href });
    window.location.assign("./overview.html");
  } else {
    if (!url.pathname.endsWith("/")) url.pathname += "/";
    const target =
      url.href === (await endpoint())
        ? new URL("./overview.html", window.location.href)
        : new URL("console/overview.html", url);
    window.location.assign(target);
  }
}

export async function getJson<T>(
  path: string,
  signal?: AbortSignal,
): Promise<T> {
  if (signal?.aborted) throw new DOMException("Aborted", "AbortError");
  if (isDesktop) {
    const result = await native<T>("api_get", { path });
    if (signal?.aborted) throw new DOMException("Aborted", "AbortError");
    return result;
  }
  const response = await fetch(new URL(path, await endpoint()), {
    signal,
    credentials: "omit",
  });
  if (!response.ok) {
    const data: unknown = await response.json().catch(() => null);
    const message =
      data && typeof data === "object" && "message" in data
        ? String(data.message)
        : `请求失败（${response.status}）`;
    throw new Error(message);
  }
  return response.json() as Promise<T>;
}

export async function postJson<T>(path: string, body?: unknown): Promise<T> {
  if (isDesktop) return native<T>("api_post", { path, body: body ?? null });
  const response = await fetch(new URL(path, await endpoint()), {
    method: "POST",
    credentials: "omit",
    headers:
      body === undefined ? undefined : { "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (!response.ok) {
    const error: unknown = await response.json().catch(() => null);
    throw new Error(
      error && typeof error === "object" && "message" in error
        ? String(error.message)
        : `操作失败（${response.status}）`,
    );
  }
  return response.json() as Promise<T>;
}

export async function saveConfiguration(revision: number): Promise<boolean> {
  if (isDesktop) return native<boolean>("save_config_export", { revision });
  const exported = await getJson<{ format_version: number; toml: string }>(
    `api/v1/config/revisions/${revision}`,
  );
  const url = URL.createObjectURL(
    new Blob([exported.toml], { type: "text/plain;charset=utf-8" }),
  );
  const link = document.createElement("a");
  link.href = url;
  link.download = `hps-config-r${revision}.toml`;
  link.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
  return true;
}
