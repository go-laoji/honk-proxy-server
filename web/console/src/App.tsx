import { useState, useEffect, lazy, Suspense, type ComponentType } from "react";
import {
  Activity,
  ArrowDownUp,
  ArrowRight,
  Box,
  Cable,
  ChevronRight,
  CircleDashed,
  Code2,
  Cpu,
  FileCode2,
  Globe2,
  Layers3,
  ListChecks,
  Menu,
  Moon,
  Network,
  PlugZap,
  Radio,
  RefreshCw,
  Settings2,
  Shield,
  Sun,
  Unplug,
  Waypoints,
} from "lucide-react";
import manifest from "../pages.manifest.json";
import { useService } from "./use-service";
import { Badge } from "./components/ui";
import Overview from "./pages/Overview";
import ConnectPage from "./pages/ConnectPage";
import SettingsPage from "./pages/SettingsPage";
import LinuxPage from "./pages/LinuxPage";
import InboundsPage from "./pages/InboundsPage";
const ProvidersPage = lazy(() => import("./pages/ProvidersPage"));
const HealthPage = lazy(() => import("./pages/HealthPage"));
const DnsPage = lazy(() => import("./pages/DnsPage"));
const GroupsPage = lazy(() => import("./pages/GroupsPage"));
const RulesPage = lazy(() => import("./pages/RulesPage"));
const NodesPage = lazy(() => import("./pages/NodesPage"));
const ConnectionsPage = lazy(() => import("./pages/ConnectionsPage"));
const ConfigPage = lazy(() => import("./pages/ConfigPage"));
const TasksPage = lazy(() => import("./pages/TasksPage"));
const EventsPage = lazy(() => import("./pages/EventsPage"));

const icons: Record<
  string,
  ComponentType<{ size?: number; className?: string }>
> = {
  index: Cable,
  overview: Activity,
  inbounds: PlugZap,
  providers: Box,
  nodes: Globe2,
  groups: Layers3,
  rules: Waypoints,
  health: Radio,
  dns: Network,
  connections: ArrowDownUp,
  events: FileCode2,
  tasks: ListChecks,
  linux: Cpu,
  config: Code2,
  settings: Settings2,
};
const groups = ["工作台", "代理网络", "运行观测", "系统管理"];
const current =
  manifest.find((page) => page.id === document.documentElement.dataset.page) ??
  manifest[0];

export default function App() {
  const service = useService();
  const [mobileOpen, setMobileOpen] = useState(false);
  const [dark, setDark] = useState(() => {
    try {
      return localStorage.getItem("hps.theme") === "dark";
    } catch {
      return false;
    }
  });
  useEffect(() => {
    document.documentElement.classList.toggle("dark", dark);
    try {
      localStorage.setItem("hps.theme", dark ? "dark" : "light");
    } catch {
      /* Theme persistence is optional. */
    }
  }, [dark]);
  const pageParent = current.group === "详情" ? "代理网络" : current.group;
  return (
    <div className="min-h-screen">
      {mobileOpen && (
        <button
          aria-label="关闭导航"
          className="fixed inset-0 z-30 bg-slate-950/50 lg:hidden"
          onClick={() => setMobileOpen(false)}
        />
      )}
      <aside
        className={`fixed inset-y-0 left-0 z-40 flex w-60 flex-col border-r border-slate-200/70 bg-white transition-transform dark:border-slate-800 dark:bg-slate-900 lg:translate-x-0 ${mobileOpen ? "translate-x-0" : "-translate-x-full"}`}
      >
        <a href="./overview.html" className="flex h-20 items-center gap-3 px-6">
          <span className="flex size-9 items-center justify-center rounded-xl bg-blue-600 text-white shadow-sm">
            <Waypoints size={23} />
          </span>
          <span>
            <strong className="block text-base tracking-tight">
              Honk Proxy
            </strong>
            <span className="muted mt-0.5 block text-[10px] tracking-[0.18em]">
              PROXY CONTROL
            </span>
          </span>
        </a>
        <nav
          aria-label="主导航"
          className="flex-1 space-y-5 overflow-y-auto px-3 py-3"
        >
          {groups.map((group) => (
            <div key={group}>
              <p className="mb-2 px-3 text-[10px] font-semibold tracking-widest text-slate-400">
                {group}
              </p>
              <div className="space-y-0.5">
                {manifest
                  .filter((page) => page.group === group)
                  .map((page) => {
                    const Icon = icons[page.id] ?? CircleDashed;
                    const active = page.id === current.id;
                    return (
                      <a
                        key={page.id}
                        href={`./${page.file}`}
                        aria-current={active ? "page" : undefined}
                        className={`flex items-center gap-3 rounded-lg px-3 py-2.5 text-[13px] font-medium transition-colors ${active ? "bg-blue-50 text-blue-700 dark:bg-blue-950 dark:text-blue-300" : "text-slate-600 hover:bg-slate-50 dark:text-slate-400 dark:hover:bg-slate-800"}`}
                      >
                        <Icon size={17} />
                        {page.title}
                        {active && (
                          <span className="ml-auto size-1.5 rounded-full bg-blue-500" />
                        )}
                      </a>
                    );
                  })}
              </div>
            </div>
          ))}
        </nav>
        <div className="m-3 rounded-xl border border-slate-200/70 bg-slate-50 p-3 dark:border-slate-700 dark:bg-slate-800/50">
          <div className="flex items-center gap-2 text-xs font-medium">
            <span
              className={`size-2 rounded-full ${service.data ? "bg-emerald-500" : "bg-slate-400"}`}
            />
            {service.data?.status.instance_name ?? "未连接服务"}
          </div>
          <p className="muted mt-2 truncate text-[10px]">
            {service.data?.endpoint ?? "选择一个服务开始管理"}
          </p>
          <a
            href="./index.html"
            className="mt-3 inline-flex items-center gap-1 text-xs text-blue-600 dark:text-blue-400"
          >
            切换连接 <ChevronRight size={12} />
          </a>
        </div>
      </aside>
      <div className="lg:pl-60">
        <header className="flex h-16 items-center justify-between border-b border-slate-200/70 bg-white/80 px-5 backdrop-blur dark:border-slate-800 dark:bg-slate-900/80 sm:px-8">
          <div className="flex items-center gap-3 text-xs">
            <button
              aria-label="打开导航"
              className="lg:hidden"
              onClick={() => setMobileOpen(true)}
            >
              <Menu size={20} />
            </button>
            <span className="muted">{pageParent}</span>
            <ChevronRight size={12} className="text-slate-400" />
            <span>{current.title}</span>
          </div>
          <div className="flex items-center gap-4">
            <span className="hidden sm:block">
              <Badge active={Boolean(service.data)}>
                {service.data
                  ? "服务已连接"
                  : service.loading
                    ? "连接中"
                    : "服务离线"}
              </Badge>
            </span>
            <button
              className="rounded-lg p-2 text-slate-500 hover:bg-slate-100 dark:hover:bg-slate-800"
              aria-label={dark ? "切换浅色模式" : "切换深色模式"}
              onClick={() => setDark(!dark)}
            >
              {dark ? <Sun size={18} /> : <Moon size={18} />}
            </button>
          </div>
        </header>
        <main className="mx-auto max-w-[1440px] p-5 sm:p-8 xl:p-10">
          <div className="mb-7 flex flex-wrap items-center justify-between gap-4">
            <div>
              <h1 className="text-2xl font-semibold tracking-tight">
                {current.title}
              </h1>
              <p className="muted mt-2 text-sm">{current.description}</p>
            </div>
            <button
              className="btn"
              disabled={service.loading}
              onClick={() => void service.refresh()}
            >
              <RefreshCw
                size={15}
                className={service.loading ? "animate-spin" : ""}
              />
              刷新状态
            </button>
          </div>
          {service.error && current.id !== "index" && (
            <div
              role="alert"
              className="panel mb-6 flex items-start gap-3 border-red-200 p-5 text-red-700 dark:border-red-900 dark:text-red-300"
            >
              <Unplug size={21} className="shrink-0" />
              <div>
                <p className="text-sm font-medium">暂时无法连接服务</p>
                <p className="mt-1 text-xs leading-6">{service.error}</p>
                <a
                  href="./index.html"
                  className="mt-2 inline-flex text-xs underline"
                >
                  检查服务地址
                </a>
              </div>
            </div>
          )}
          {current.id === "index" ? (
            <ConnectPage data={service.data} />
          ) : service.loading && !service.data ? (
            <div className="panel flex items-center justify-center gap-3 p-16 text-sm text-slate-500">
              <RefreshCw size={18} className="animate-spin" />
              正在读取服务状态…
            </div>
          ) : service.data ? (
            current.id === "overview" ? (
              <Overview data={service.data} />
            ) : current.id === "config" ? (
              <Suspense fallback={<p className="muted">正在打开配置工作台…</p>}>
                <ConfigPage refreshService={service.refresh} />
              </Suspense>
            ) : ["providers", "provider-detail"].includes(current.id) ? (
              <Suspense fallback={<p className="muted">正在读取供应商…</p>}>
                <ProvidersPage
                  detailPage={current.id === "provider-detail"}
                  serviceEndpoint={service.data.endpoint}
                  serviceEpoch={service.data.status.instance_id}
                  refreshService={service.refresh}
                />
              </Suspense>
            ) : current.id === "health" ? (
              <Suspense fallback={<p className="muted">正在读取健康检测…</p>}>
                <HealthPage
                  key={service.data.endpoint}
                  serviceEndpoint={service.data.endpoint}
                  serviceEpoch={service.data.status.instance_id}
                  refreshService={service.refresh}
                />
              </Suspense>
            ) : current.id === "dns" ? (
              <Suspense fallback={<p className="muted">正在读取 DNS…</p>}>
                <DnsPage refreshService={service.refresh} />
              </Suspense>
            ) : current.id === "groups" ? (
              <Suspense fallback={<p className="muted">正在读取分组…</p>}>
                <GroupsPage
                  key={service.data.endpoint}
                  refreshService={service.refresh}
                />
              </Suspense>
            ) : current.id === "rules" ? (
              <Suspense fallback={<p className="muted">正在读取规则…</p>}>
                <RulesPage
                  key={service.data.endpoint}
                  refreshService={service.refresh}
                />
              </Suspense>
            ) : current.id === "nodes" || current.id === "node-detail" ? (
              <Suspense fallback={<p className="muted">正在读取节点…</p>}>
                <NodesPage key={service.data.endpoint} />
              </Suspense>
            ) : current.id === "connections" ? (
              <Suspense fallback={<p className="muted">正在读取连接…</p>}>
                <ConnectionsPage key={service.data.endpoint} />
              </Suspense>
            ) : current.id === "tasks" ? (
              <Suspense fallback={<p className="muted">正在读取后台任务…</p>}>
                <TasksPage key={service.data.endpoint} />
              </Suspense>
            ) : current.id === "events" ? (
              <Suspense fallback={<p className="muted">正在读取服务事件…</p>}>
                <EventsPage key={service.data.endpoint} />
              </Suspense>
            ) : current.id === "settings" ? (
              <SettingsPage data={service.data} />
            ) : current.id === "linux" ? (
              <LinuxPage data={service.data} refreshService={service.refresh} />
            ) : current.id === "inbounds" ? (
              <InboundsPage data={service.data} refresh={service.refresh} />
            ) : (
              <section className="panel px-6 py-16 text-center">
                <div className="mx-auto mb-5 flex size-14 items-center justify-center rounded-2xl bg-slate-50 text-slate-400 dark:bg-slate-800">
                  <CircleDashed size={30} />
                </div>
                <h2 className="font-semibold">{current.title}当前不可用</h2>
                <p className="muted mx-auto mt-3 max-w-md text-sm leading-7">
                  当前服务未提供此模块的操作能力。你可以查看运行概览，确认服务已连接的功能与实际状态。
                </p>
                <a href="./overview.html" className="btn mt-6">
                  查看运行概览 <ArrowRight size={15} />
                </a>
              </section>
            )
          ) : null}
          <footer className="muted mt-10 flex flex-wrap items-center justify-between gap-2 border-t border-slate-200/70 pt-5 text-[11px] dark:border-slate-800">
            <span>Honk Proxy · 统一代理控制台</span>
            <span className="flex items-center gap-1.5">
              <Shield size={12} />
              全部功能直接访问，无需账号
            </span>
          </footer>
        </main>
      </div>
    </div>
  );
}
