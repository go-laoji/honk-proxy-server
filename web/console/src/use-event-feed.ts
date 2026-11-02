import { useEffect, useRef, useState } from "react";
import { getJson } from "./platform";
import { watchEvents } from "./event-transport";
import {
  queryString,
  validateEventPage,
  type EventFrame,
  type EventQuery,
  type FeedState,
} from "./tasks";

/** Snapshot before subscribing prevents lost changes, including while a task list is being fetched. */
export function useEventFeed(
  query: EventQuery,
  enabled: boolean,
  onFrame: (frame: EventFrame) => void,
  expectedEpoch?: string,
) {
  const [state, setState] = useState<FeedState>({ state: "connecting" });
  const [restart, setRestart] = useState(0);
  const [visible, setVisible] = useState(!document.hidden);
  const handler = useRef(onFrame);
  handler.current = onFrame;
  const latestQuery = useRef(query);
  latestQuery.current = query;
  const lastRead = useRef<{ key: string; restart: number } | null>(null);
  const checkpoint = useRef<{ key: string; cursor: string } | null>(null);
  const key = `${queryString(query)}|${expectedEpoch ?? ""}`;
  useEffect(() => {
    const visibility = () => setVisible(!document.hidden);
    const hide = () => setVisible(false);
    document.addEventListener("visibilitychange", visibility);
    window.addEventListener("pagehide", hide);
    window.addEventListener("pageshow", visibility);
    return () => {
      document.removeEventListener("visibilitychange", visibility);
      window.removeEventListener("pagehide", hide);
      window.removeEventListener("pageshow", visibility);
    };
  }, []);
  useEffect(() => {
    if (!visible) {
      setState({ state: "hidden" });
      return;
    }
    if (
      !enabled &&
      checkpoint.current?.key === key &&
      lastRead.current?.key === key &&
      lastRead.current.restart === restart
    ) {
      setState({ state: "paused" });
      return;
    }
    const query = latestQuery.current;
    lastRead.current = { key, restart };
    let disposed = false;
    let close: (() => void) | undefined;
    const controller = new AbortController();
    const request = {
      ...query,
      cursor:
        checkpoint.current?.key === key
          ? checkpoint.current.cursor
          : query.cursor,
    };
    function receive(frame: EventFrame) {
      if (disposed) return;
      if (frame.kind !== "unavailable") {
        if (expectedEpoch && frame.data.epoch !== expectedEpoch)
          throw new Error(
            "关联链接所属的服务实例已变化；清除链接中的实例限制后重新查询",
          );
        handler.current(frame);
        checkpoint.current = { key, cursor: frame.data.cursor };
      } else handler.current(frame);
    }
    setState({ state: "connecting" });
    const search = queryString(request);
    void getJson<unknown>(
      `api/v1/events${search ? `?${search}` : ""}`,
      controller.signal,
    )
      .then((body) => {
        if (disposed) return;
        const page = validateEventPage(body);
        receive({
          kind: page.reset_reason
            ? "reset"
            : request.cursor
              ? "events"
              : "snapshot",
          data: page,
        });
        if (enabled)
          close = watchEvents(
            { ...query, cursor: page.cursor },
            {
              frame: receive,
              state: (next) => {
                if (!disposed) setState(next);
              },
            },
          );
        else setState({ state: "paused" });
      })
      .catch((cause) => {
        if (!disposed)
          setState({
            state: "error",
            message: cause instanceof Error ? cause.message : String(cause),
          });
      });
    return () => {
      disposed = true;
      controller.abort();
      close?.();
    };
  }, [key, enabled, visible, restart, expectedEpoch]);
  return { state, reconnect: () => setRestart((n) => n + 1) };
}
