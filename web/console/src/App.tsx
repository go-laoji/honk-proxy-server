import {
  useState,
  useEffect,
  useRef,
  lazy,
  Suspense,
  type ComponentType,
} from "react";
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
  X,
} from "lucide-react";
import manifest from "../pages.manifest.json";
import { useService, type ServiceData } from "./use-service";
import { Badge } from "./components/ui";
import BrandLogo from "./components/BrandLogo";
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
const groups = ["工作台", "运行观测", "代理与路由", "系统管理"].map(
  (title) => ({
    title,
    pages: manifest.filter((page) => page.group === title),
  }),
);
const current =
  manifest.find((page) => page.id === document.documentElement.dataset.page) ??
  manifest[0];

const parent = manifest.find((page) => page.id === current.parent);
const currentSection = parent ?? current;
const CurrentIcon = icons[currentSection.id] ?? CircleDashed;

function Navigation({ data }: { data: ServiceData | null }) {
  return (
    <>
      <a href="./overview.html" className="brand-link">
        <BrandLogo />
        <span>
          <strong className="block text-base tracking-tight">Honk Proxy</strong>
          <span className="muted block text-[11px]">代理控制台</span>
        </span>
      </a>
      <nav
        aria-label="主导航"
        className="min-h-0 flex-1 overflow-y-auto px-3 py-2"
      >
        {groups.map(({ title, pages }) => (
          <div key={title} className="nav-group">
            <p className="nav-group-label">{title}</p>
            {pages.map((page) => {
              const Icon = icons[page.id] ?? CircleDashed;
              const active = page.id === currentSection.id;
              return (
                <a
                  key={page.id}
                  href={`./${page.file}`}
                  aria-current={
                    active ? (parent ? "location" : "page") : undefined
                  }
                  className="nav-link"
                >
                  <span className="nav-icon" aria-hidden="true">
                    <Icon size={17} />
                  </span>
                  {page.title}
                </a>
              );
            })}
          </div>
        ))}
      </nav>
      <div className="sidebar-instance">
        <div className="flex items-center gap-2 text-xs font-semibold">
          <span
            className="instance-dot"
            data-connected={Boolean(data)}
            aria-hidden="true"
          />
          <span className="truncate">
            {data?.status.instance_name ?? "未连接服务"}
          </span>
        </div>
        <p className="muted mt-1 truncate text-[11px]" title={data?.endpoint}>
          {data?.endpoint ?? "选择一个服务开始管理"}
        </p>
        <a href="./index.html" className="text-link mt-2">
          切换连接 <ChevronRight size={13} aria-hidden="true" />
        </a>
      </div>
    </>
  );
}

export default function App() {
  const service = useService();
  const mobileNav = useRef<HTMLDialogElement>(null);
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
  useEffect(() => {
    const desktop = window.matchMedia("(min-width: 64rem)");
    const closeOnDesktop = () => {
      if (desktop.matches) mobileNav.current?.close();
    };
    desktop.addEventListener("change", closeOnDesktop);
    return () => desktop.removeEventListener("change", closeOnDesktop);
  }, []);
  return (
    <div className="app-shell min-h-screen">
      <aside className="sidebar desktop-sidebar">
        <Navigation data={service.data} />
      </aside>
      <dialog
        id="mobile-navigation"
        ref={mobileNav}
        className="sidebar mobile-sidebar"
        aria-label="导航菜单"
        onClose={() => setMobileOpen(false)}
        onClick={(event) => {
          if (event.target === event.currentTarget) event.currentTarget.close();
        }}
      >
        <div className="relative flex h-full flex-col">
          <button
            type="button"
            className="icon-button absolute right-3 top-3"
            aria-label="关闭导航"
            onClick={() => mobileNav.current?.close()}
          >
            <X size={19} aria-hidden="true" />
          </button>
          {mobileOpen && <Navigation data={service.data} />}
        </div>
      </dialog>
      <div className="app-content min-w-0">
        <header className="app-header">
          <div className="flex min-w-0 items-center gap-2 text-xs sm:gap-3">
            <button
              type="button"
              aria-label="打开导航"
              aria-controls="mobile-navigation"
              aria-expanded={mobileOpen}
              className="icon-button lg:hidden"
              onClick={() => {
                mobileNav.current?.showModal();
                setMobileOpen(true);
              }}
            >
              <Menu size={20} aria-hidden="true" />
            </button>
            <span className="muted hidden sm:inline">
              {currentSection.group}
            </span>
            <ChevronRight
              size={13}
              className="muted hidden sm:block"
              aria-hidden="true"
            />
            <span className="truncate font-medium">{current.title}</span>
          </div>
          <div className="flex shrink-0 items-center gap-2 sm:gap-3">
            <span role="status">
              <Badge active={Boolean(service.data)}>
                {service.data
                  ? "服务已连接"
                  : service.loading
                    ? "连接中"
                    : "服务离线"}
              </Badge>
            </span>
            <button
              className="icon-button"
              aria-label={dark ? "切换浅色模式" : "切换深色模式"}
              onClick={() => setDark((value) => !value)}
            >
              <span className="theme-icon inline-flex" aria-hidden="true">
                {dark ? <Sun size={18} /> : <Moon size={18} />}
              </span>
            </button>
          </div>
        </header>
        <main className="px-3 py-3 sm:px-4 xl:px-5">
          <div className="page-heading">
            <div className="flex min-w-0 items-center gap-2.5">
              <span className="page-heading-mark" aria-hidden="true">
                <CurrentIcon size={18} />
              </span>
              <div className="min-w-0">
                <h1 className="text-lg leading-tight font-semibold tracking-tight sm:text-xl">
                  {current.title}
                </h1>
                <p className="muted mt-0.5 text-xs">{current.description}</p>
              </div>
            </div>
            <button
              className="btn"
              disabled={service.loading}
              onClick={() => void service.refresh()}
            >
              <span
                className="action-icon inline-flex"
                data-spinning={service.loading}
                aria-hidden="true"
              >
                <RefreshCw size={15} />
              </span>
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
            <ConnectPage data={service.data} refreshService={service.refresh} />
          ) : service.loading && !service.data ? (
            <div className="panel flex items-center justify-center gap-3 p-16 text-sm text-slate-500">
              <span
                className="action-icon inline-flex"
                data-spinning="true"
                aria-hidden="true"
              >
                <RefreshCw size={18} />
              </span>
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
              <SettingsPage
                data={service.data}
                refreshService={service.refresh}
              />
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
          <footer className="muted mt-6 flex flex-wrap items-center justify-between gap-2 border-t border-slate-200/70 pt-3 text-[11px] dark:border-slate-800">
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
