import type { ServiceData } from "../use-service";
import { Details } from "../components/ui";
import { isDesktop } from "../platform";

export default function SettingsPage({ data }: { data: ServiceData }) {
  return (
    <div className="grid gap-6 lg:grid-cols-2">
      <section className="panel">
        <h2 className="border-b border-slate-100 p-5 font-semibold dark:border-slate-800">
          当前启动配置
        </h2>
        <Details
          rows={[
            ["实例名称", data.config.instance_name],
            ["管理监听", <code>{data.config.management_listen}</code>],
            ["日志过滤", <code>{data.config.log_filter}</code>],
            ["访问方式", "无需登录，直接访问全部功能"],
          ]}
        />
        <p className="muted px-5 pb-5 text-xs leading-6">
          启动参数通过 TOML 配置文件设置，重新启动服务后生效。
        </p>
      </section>
      <section className="panel">
        <h2 className="border-b border-slate-100 p-5 font-semibold dark:border-slate-800">
          版本与构建
        </h2>
        <Details
          rows={[
            ["服务", data.status.version.service],
            ["版本", data.status.version.version],
            ["API", data.status.version.api_version],
            [
              "Web 构建标识",
              <code className="text-xs">
                {data.status.version.web_build_id ?? "无"}
              </code>,
            ],
            ["客户端", isDesktop ? "Desktop" : "Web 浏览器"],
          ]}
        />
      </section>
    </div>
  );
}
