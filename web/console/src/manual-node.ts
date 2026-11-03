export type ManualProtocol = "http" | "https" | "socks5";
export type ManualNode = {
  id: string;
  protocol: ManualProtocol;
  host: string;
  port: string;
  username: string;
  password: string;
  enabled: boolean;
  expires: string;
};

export const defaultPort = (protocol: ManualProtocol) =>
  protocol === "https" ? "443" : protocol === "socks5" ? "1080" : "80";

export function readProxyUrl(value: string): Partial<ManualNode> {
  try {
    const url = new URL(value.trim());
    const scheme = url.protocol.slice(0, -1);
    if (
      !["http", "https", "socks5", "socks5h"].includes(scheme) ||
      !url.hostname ||
      !["", "/"].includes(url.pathname) ||
      url.search ||
      url.hash
    )
      throw new Error();
    const protocol = (
      scheme === "socks5h" ? "socks5" : scheme
    ) as ManualProtocol;
    return {
      protocol,
      host: url.hostname.replace(/^\[(.*)\]$/, "$1"),
      port: url.port || defaultPort(protocol),
      username: decodeURIComponent(url.username),
      password: decodeURIComponent(url.password),
    };
  } catch {
    throw new Error(
      "请输入有效的 HTTP、HTTPS 或 SOCKS5 代理链接，不支持路径、查询参数和片段。",
    );
  }
}

export function manualNodeDocument(form: ManualNode, generatedId: string) {
  const id = form.id.trim() || generatedId;
  if (!/^[A-Za-z0-9_-]{1,64}$/.test(id))
    throw new Error("节点标识请使用 1～64 个字母、数字、下划线或连字符。");
  const host = form.host.trim().replace(/^\[(.*)\]$/, "$1");
  if (!host || /[\s/@?#\\]/.test(host))
    throw new Error("请填写 IP 或域名；协议、端口和认证信息请使用对应字段。");
  const port = Number(form.port);
  if (
    !/^\d+$/.test(form.port) ||
    !Number.isInteger(port) ||
    port < 1 ||
    port > 65535
  )
    throw new Error("端口必须是 1～65535 的整数。");
  const hasCredentials = !!(form.username || form.password);
  const bytes = new TextEncoder();
  if (
    hasCredentials &&
    (!form.username ||
      form.username.includes(":") ||
      bytes.encode(form.username).length > 255 ||
      bytes.encode(form.password).length > 255 ||
      (form.protocol === "socks5" && !form.password))
  )
    throw new Error(
      "请填写有效的上游用户名和密码（各最多 255 字节，用户名不能包含冒号）；SOCKS5 认证密码不能为空。",
    );
  const expires = form.expires
    ? Math.floor(new Date(form.expires).getTime() / 1000)
    : null;
  if (
    expires !== null &&
    (!Number.isFinite(expires) || expires <= Date.now() / 1000)
  )
    throw new Error("到期时间必须晚于当前时间；长效节点可以留空。");
  return {
    id,
    document: JSON.stringify([
      {
        id,
        protocol: form.protocol,
        host,
        port,
        enabled: form.enabled,
        credentials: hasCredentials
          ? { username: form.username, password: form.password }
          : null,
        expires_at_unix: expires,
        ca_certificate: null,
        tls_server_name: null,
      },
    ]),
  };
}
