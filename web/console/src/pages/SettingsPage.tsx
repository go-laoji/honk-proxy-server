import type { ServiceData } from "../use-service";
import { Details } from "../components/ui";
import { isDesktop } from "../platform";
import { getJson } from "../platform";
import { useEffect, useState } from "react";
import type { ConfigWorkspace } from "../types";
import LocalSystemPanel from "../components/LocalSystemPanel";

export default function SettingsPage({
  data,
  refreshService,
}: {
  data: ServiceData;
  refreshService: () => Promise<void>;
}) {
  const [workspace, setWorkspace] = useState<ConfigWorkspace | null>(null);
  useEffect(() => {
    const controller = new AbortController();
    void getJson<ConfigWorkspace>("api/v1/config/workspace", controller.signal)
      .then(setWorkspace)
      .catch(() => {});
    return () => controller.abort();
  }, []);
  return (
    <div className="grid gap-6 lg:grid-cols-2">
      {isDesktop && (
        <div className="lg:col-span-2">
          <LocalSystemPanel data={data} refreshService={refreshService} />
        </div>
      )}
      <section className="panel">
        <h2 className="border-b border-slate-100 p-5 font-semibold dark:border-slate-800">
          当前运行配置
        </h2>
        <Details
          rows={[
            ["实例名称", data.config.instance_name],
            ["管理监听", <code>{data.config.management_listen}</code>],
            ["日志过滤", <code>{data.config.log_filter}</code>],
            [
              "管理 TLS",
              data.config.management_tls ? "HTTPS · 无客户端账号认证" : "HTTP",
            ],
            ["实际服务地址", data.endpoint],
            ["存储位置", workspace?.storage_path ?? "临时内存或正在读取"],
            [
              "配置保留上限",
              data.config.retention
                ? `${data.config.retention.config_versions} 个版本，运行及引用版本额外保留`
                : "未提供",
            ],
            [
              "活动 / 任务保留期",
              data.config.retention
                ? `${data.config.retention.activity_days} / ${data.config.retention.task_days} 天`
                : "未提供",
            ],
            ...(workspace?.management_certificate
              ? ([
                  [
                    "管理证书到期",
                    new Date(
                      workspace.management_certificate.not_after_unix * 1000,
                    ).toLocaleString("zh-CN"),
                  ],
                ] as [string, string][])
              : []),
            ["访问方式", "无需登录，直接访问全部功能"],
          ]}
        />
        <p className="muted px-5 pb-5 text-xs leading-6">
          管理监听、TLS 和日志过滤修改后需重启；代理配置可在线激活。数据目录通过
          Daemon 的 --data-dir
          指定，不自动搬迁数据库。活动与任务模块按配置的天数保留。
        </p>
        <a className="btn mx-5 mb-5" href="./config.html">
          打开配置工作台
        </a>
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
