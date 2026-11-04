import { ArrowRight, CircleHelp } from "lucide-react";
import type { ServiceData } from "../use-service";
import { Badge, Details } from "../components/ui";
import NodeSummary from "../components/NodeSummary";
import ConnectionSummary from "../components/ConnectionSummary";
import ObservationPanel from "../components/ObservationPanel";

export default function Overview({ data }: { data: ServiceData }) {
  const { status, capabilities } = data;
  const seconds = status.uptime_seconds;
  const uptime =
    seconds < 60
      ? `${seconds} 秒`
      : seconds < 3600
        ? `${Math.floor(seconds / 60)} 分钟`
        : seconds < 86400
          ? `${Math.floor(seconds / 3600)} 小时 ${Math.floor((seconds % 3600) / 60)} 分钟`
          : `${Math.floor(seconds / 86400)} 天 ${Math.floor((seconds % 86400) / 3600)} 小时`;
  return (
    <div className="space-y-5">
      <ConnectionSummary
        key={`connections:${data.endpoint}`}
        endpoint={data.endpoint}
      />
      <div className="grid items-start gap-4 xl:grid-cols-[minmax(0,2fr)_minmax(16rem,1fr)]">
        <ObservationPanel key={data.endpoint} />
        <section className="panel p-4 sm:p-5" aria-labelledby="runtime-heading">
          <h2 id="runtime-heading" className="font-semibold">
            运行状态
          </h2>
          <dl className="dashboard-status mt-2">
            <div>
              <dt>管理服务</dt>
              <dd>
                <Badge active={status.management_ready}>
                  {status.management_ready ? "运行中" : "未就绪"}
                </Badge>
              </dd>
            </div>
            <div>
              <dt>代理数据面</dt>
              <dd>
                <Badge active={status.proxy_ready}>
                  {status.proxy_ready ? "已就绪" : "未就绪"}
                </Badge>
              </dd>
            </div>
            <div>
              <dt>Linux eBPF</dt>
              <dd>
                <Badge active={capabilities.linux_ebpf.ready}>
                  {capabilities.linux_ebpf.ready
                    ? "已接管"
                    : capabilities.linux_ebpf.platform_supported
                      ? "未启用"
                      : "当前平台不可用"}
                </Badge>
              </dd>
            </div>
            <div>
              <dt>持续运行</dt>
              <dd className="tabular-nums">{uptime}</dd>
            </div>
          </dl>
          {!status.proxy_ready && (
            <div className="runtime-notice mt-3 flex items-start gap-2 rounded-lg p-3">
              <CircleHelp
                className="mt-0.5 shrink-0"
                size={16}
                aria-hidden="true"
              />
              <div>
                <p className="text-xs font-semibold">代理服务尚未就绪</p>
                <p className="mt-1 text-xs leading-5">
                  管理连接已建立。请检查代理入口和转发路径。
                </p>
              </div>
            </div>
          )}
          <a href="./inbounds.html" className="text-link mt-4">
            查看代理入口 <ArrowRight size={14} aria-hidden="true" />
          </a>
        </section>
      </div>
      <NodeSummary key={`nodes:${data.endpoint}`} endpoint={data.endpoint} />
      <div className="grid gap-4 xl:grid-cols-5">
        <section className="panel xl:col-span-3">
          <div className="flex items-center justify-between border-b border-slate-100 p-5 dark:border-slate-800">
            <h2 className="font-semibold">当前服务</h2>
            <Badge active={status.management_ready}>
              {status.management_ready ? "连接正常" : "未就绪"}
            </Badge>
          </div>
          <Details
            rows={[
              ["实例名称", status.instance_name],
              ["服务地址", data.endpoint],
              [
                "实例标识",
                <code className="text-xs">{status.instance_id}</code>,
              ],
              ["运行平台", status.platform],
              ["服务版本", status.version.version],
              ["API 版本", status.version.api_version],
              [
                "Web 构建",
                <code className="text-xs">
                  {status.version.web_build_id?.slice(0, 16) ??
                    "未包含控制台资源"}
                </code>,
              ],
            ]}
          />
        </section>
        <section className="panel xl:col-span-2">
          <div className="border-b border-slate-100 p-5 dark:border-slate-800">
            <h2 className="font-semibold">代理协议能力</h2>
            <p className="muted mt-1 text-xs">读取服务实际返回的可用状态</p>
          </div>
          <div className="space-y-5 p-5">
            {[
              ["HTTP / CONNECT", capabilities.http_proxy.available],
              ["HTTPS 代理", capabilities.https_proxy.available],
              ["SOCKS5 TCP", capabilities.socks5_proxy.available],
              [
                "SOCKS5 UDP ASSOCIATE",
                capabilities.socks5_udp?.available ?? false,
              ],
            ].map(([label, enabled]) => (
              <div
                key={String(label)}
                className="flex items-center justify-between"
              >
                <span className="text-sm">{label}</span>
                <Badge active={Boolean(enabled)}>
                  {enabled ? "可用" : "不可用"}
                </Badge>
              </div>
            ))}
          </div>
        </section>
      </div>
    </div>
  );
}
