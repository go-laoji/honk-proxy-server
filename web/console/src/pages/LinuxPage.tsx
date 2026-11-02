import { Cpu } from "lucide-react";
import type { ServiceData } from "../use-service";
import { Details } from "../components/ui";

export default function LinuxPage({ data }: { data: ServiceData }) {
  const ebpf = data.capabilities.linux_ebpf;
  return (
    <section className="panel">
      <div className="flex items-start gap-4 border-b border-slate-100 p-6 dark:border-slate-800">
        <div className="rounded-xl bg-blue-50 p-3 text-blue-600 dark:bg-blue-950">
          <Cpu size={26} />
        </div>
        <div>
          <h2 className="font-semibold">真实数据面状态</h2>
          <p className="muted mt-2 text-sm leading-6">{ebpf.reason}</p>
        </div>
      </div>
      <Details
        rows={[
          ["目标服务平台", data.status.platform],
          [
            "平台支持",
            ebpf.platform_supported
              ? "Linux 平台，仍需检查内核条件"
              : "当前平台不支持",
          ],
          ["已编译 eBPF", ebpf.compiled ? "是" : "否"],
          ["已配置", ebpf.configured ? "是" : "否"],
          ["已挂载", ebpf.attached ? "是" : "否"],
          ["接管就绪", ebpf.ready ? "是" : "否"],
        ]}
      />
    </section>
  );
}
