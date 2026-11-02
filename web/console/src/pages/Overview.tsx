import {
  ArrowRight,
  Server,
  Waypoints,
  Cpu,
  Clock3,
  CircleHelp,
  Layers3,
} from "lucide-react";
import type { ServiceData } from "../use-service";
import { Badge, Details, Stat } from "../components/ui";
import NodeSummary from "../components/NodeSummary";
import ConnectionSummary from "../components/ConnectionSummary";
import ObservationPanel from "../components/ObservationPanel";

export default function Overview({ data }: { data: ServiceData }) {
  const { status, capabilities } = data;
  const uptime =
    status.uptime_seconds < 60
      ? `${status.uptime_seconds} 秒`
      : `${Math.floor(status.uptime_seconds / 60)} 分钟`;
  return (
    <>
      <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
        <Stat
          title="管理服务"
          value={status.management_ready ? "运行中" : "未就绪"}
          detail="CLI、浏览器与 Desktop 共用服务"
          icon={Server}
        />
        <Stat
          title="代理数据面"
          value={status.proxy_ready ? "已就绪" : "未就绪"}
          detail="以实际代理引擎状态为准"
          icon={Waypoints}
        />
        <Stat
          title="Linux eBPF"
          value={capabilities.linux_ebpf.ready ? "已接管" : "未启用"}
          detail={
            capabilities.linux_ebpf.platform_supported
              ? "当前服务运行于 Linux"
              : "当前服务不在 Linux 上运行"
          }
          icon={Cpu}
        />
        <Stat
          title="持续运行"
          value={uptime}
          detail={`服务版本 ${status.version.version} · ${status.platform}`}
          icon={Clock3}
        />
      </div>
      <NodeSummary endpoint={data.endpoint} />
      <ConnectionSummary endpoint={data.endpoint} />
      <div className="mt-6">
        <ObservationPanel key={data.endpoint} />
      </div>
      {!status.proxy_ready && (
        <div className="mt-6 flex items-start gap-3 rounded-xl border border-amber-200/70 bg-amber-50 px-5 py-4 text-sm text-amber-900 dark:border-amber-900 dark:bg-amber-950/40 dark:text-amber-200">
          <CircleHelp className="mt-0.5 shrink-0" size={19} />
          <div>
            <p className="font-medium">代理服务尚未就绪</p>
            <p className="mt-1 text-xs leading-6 opacity-80">
              管理连接已建立，当前服务还不能转发代理流量。请根据实际能力状态配置和启用代理入口。
            </p>
          </div>
        </div>
      )}
      <div className="mt-6 grid gap-6 xl:grid-cols-5">
        <section className="panel xl:col-span-3">
          <div className="flex items-center justify-between border-b border-slate-100 p-5 dark:border-slate-800">
            <h2 className="font-semibold">当前服务</h2>
            <Badge active={status.management_ready}>连接正常</Badge>
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
            <a
              href="./inbounds.html"
              className="flex items-center gap-1 pt-3 text-sm font-medium text-blue-600 dark:text-blue-400"
            >
              查看代理入口 <ArrowRight size={15} />
            </a>
          </div>
        </section>
      </div>
      <section className="panel mt-6 flex flex-col gap-4 p-5 sm:flex-row sm:items-center sm:justify-between">
        <div className="flex items-center gap-3">
          <div className="rounded-xl bg-blue-50 p-3 text-blue-600 dark:bg-blue-950">
            <Layers3 size={22} />
          </div>
          <div>
            <h2 className="text-sm font-semibold">
              一个入口，统一管理代理出口
            </h2>
            <p className="muted mt-1 text-xs">
              短效供应商、租约与路由状态由同一服务管理
            </p>
          </div>
        </div>
        <a className="btn" href="./providers.html">
          查看上游供应商 <ArrowRight size={15} />
        </a>
      </section>
    </>
  );
}
