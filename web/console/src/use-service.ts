import { useCallback, useEffect, useRef, useState } from "react";
import { endpoint, getJson } from "./platform";
import type { Capabilities, DaemonConfig, ServiceStatus } from "./types";

export interface ServiceData {
  endpoint: string;
  status: ServiceStatus;
  capabilities: Capabilities;
  config: DaemonConfig;
}

export function useService() {
  const [data, setData] = useState<ServiceData | null>(null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(true);
  const controller = useRef<AbortController | null>(null);
  const refresh = useCallback(async () => {
    controller.current?.abort();
    const current = new AbortController();
    controller.current = current;
    setLoading(true);
    setError("");
    try {
      const [address, status, capabilities, config] = await Promise.all([
        endpoint(),
        getJson<ServiceStatus>("api/v1/status", current.signal),
        getJson<Capabilities>("api/v1/capabilities", current.signal),
        getJson<DaemonConfig>("api/v1/config", current.signal),
      ]);
      if (
        status?.version?.api_version !== "v1" ||
        !capabilities?.linux_ebpf ||
        typeof config?.management_listen !== "string"
      ) {
        throw new Error(
          "目标服务返回了不兼容的数据，请检查服务地址与 API 版本",
        );
      }
      if (!current.signal.aborted)
        setData({ endpoint: address, status, capabilities, config });
    } catch (cause) {
      if (!current.signal.aborted) {
        setData(null);
        setError(cause instanceof Error ? cause.message : String(cause));
      }
    } finally {
      if (!current.signal.aborted) setLoading(false);
    }
  }, []);
  useEffect(() => {
    void refresh();
    return () => controller.current?.abort();
  }, [refresh]);
  return { data, error, loading, refresh };
}
