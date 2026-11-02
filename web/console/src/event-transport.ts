import { endpoint, isDesktop, native, nativeRequestContext } from "./platform";
import {
  cursorParts,
  queryString,
  validateEventPage,
  type EventFrame,
  type EventQuery,
  type FeedState,
} from "./tasks";

type NativeContext = { target_generation: number; document_generation: number };
type Identity = { context: NativeContext; serial: number; id: string };
type NativeMessage = Identity & {
  sequence: number;
  payload:
    | { type: "frame"; value: EventFrame }
    | { type: "error"; value: { message: string; code: string } };
};
type Handlers = {
  frame: (value: EventFrame) => void;
  state: (value: FeedState) => void;
};
let serial = 0;

/** One subscription and one reconnect timer. Disposal cancels native IO and rejects late deliveries. */
export function watchEvents(input: EventQuery, handlers: Handlers): () => void {
  let closed = false,
    attempt = 0,
    generation = 0;
  let cursor = input.cursor;
  let stopCurrent: (() => void) | null = null;
  let timer: ReturnType<typeof setTimeout> | null = null;
  function stop() {
    stopCurrent?.();
    stopCurrent = null;
  }
  function fail(message: string) {
    stop();
    generation++;
    handlers.state({ state: "error", message });
  }
  function retry(message: string) {
    stop();
    if (closed) return;
    generation++;
    if (attempt >= 5) {
      handlers.state({ state: "error", message });
      return;
    }
    attempt++;
    handlers.state({ state: "reconnecting", attempt, message });
    timer = setTimeout(
      () => {
        timer = null;
        void open();
      },
      Math.min(500 * 2 ** (attempt - 1), 8000),
    );
  }
  function accept(frame: EventFrame, epoch: { value?: string }) {
    if (frame.kind === "unavailable") {
      retry("事件来源暂不可用");
      return;
    }
    const page = validateEventPage(frame.data);
    const next = cursorParts(page.cursor)!;
    if (epoch.value && epoch.value !== page.epoch)
      throw new Error("同一订阅混入了其他实例的事件");
    if (
      (frame.kind === "reset") !== (page.reset_reason !== null) ||
      (frame.kind === "checkpoint" && page.records.length)
    )
      throw new Error("事件帧与快照状态不一致");
    const prior = cursorParts(cursor);
    if (
      frame.kind !== "reset" &&
      prior?.epoch === next.epoch &&
      next.sequence < prior.sequence
    )
      throw new Error("事件游标发生倒退");
    handlers.frame(frame);
    epoch.value = page.epoch;
    cursor = page.cursor;
    attempt = 0;
    handlers.state({ state: "live" });
  }
  async function open() {
    if (closed) return;
    const current = ++generation;
    const epoch: { value?: string } = {};
    handlers.state({ state: "connecting" });
    try {
      if (isDesktop) {
        const { Channel } = await import("@tauri-apps/api/core");
        const subscriptionSerial = ++serial;
        const id = crypto.randomUUID();
        const identity: Identity = {
          context: await nativeRequestContext(),
          serial: subscriptionSerial,
          id,
        };
        if (closed || current !== generation) return;
        const channel = new Channel<NativeMessage>();
        let lastSequence = 0;
        const cancel = () => {
          void native("events_cancel", identity).catch(() => undefined);
        };
        stopCurrent = cancel;
        channel.onmessage = (message) => {
          if (
            closed ||
            current !== generation ||
            message.id !== identity.id ||
            message.serial !== identity.serial ||
            message.context.target_generation !==
              identity.context.target_generation ||
            message.context.document_generation !==
              identity.context.document_generation ||
            message.sequence <= lastSequence
          )
            return;
          lastSequence = message.sequence;
          if (message.payload.type === "error") {
            retry(message.payload.value.message);
            return;
          }
          try {
            accept(message.payload.value, epoch);
            if (!closed && current === generation)
              void native("events_ack", {
                ...identity,
                sequence: message.sequence,
              }).catch(() => {
                if (!closed && current === generation)
                  retry("原生事件确认失败");
              });
          } catch (cause) {
            fail(cause instanceof Error ? cause.message : "原生事件数据无效");
          }
        };
        await native("events_subscribe", {
          request: {
            ...identity,
            query: Object.fromEntries(
              Object.entries({ ...input, cursor }).filter(
                ([, v]) => v !== undefined && v !== "",
              ),
            ),
          },
          channel,
        });
        if (closed || current !== generation) cancel();
      } else {
        const base = await endpoint();
        if (closed || current !== generation) return;
        const path = `api/v1/events/stream?${queryString({ ...input, cursor })}`;
        const source = new EventSource(new URL(path, base), {
          withCredentials: false,
        });
        stopCurrent = () => source.close();
        for (const kind of [
          "snapshot",
          "events",
          "checkpoint",
          "reset",
          "unavailable",
        ] as const) {
          source.addEventListener(kind, (event: MessageEvent<string>) => {
            if (closed || current !== generation) return;
            try {
              if (event.data.length > 512 * 1024)
                throw new Error("事件帧超过大小限制");
              const data: unknown = JSON.parse(event.data);
              if (kind === "unavailable") {
                retry("事件来源暂不可用");
                return;
              }
              const page = validateEventPage(data);
              if (event.lastEventId !== page.cursor)
                throw new Error("事件帧的游标不一致");
              accept({ kind, data: page }, epoch);
            } catch (cause) {
              fail(cause instanceof Error ? cause.message : "事件数据无效");
            }
          });
        }
        source.onerror = () => {
          if (!closed && current === generation)
            retry("事件连接中断，正在从最后完整游标恢复");
        };
      }
    } catch (cause) {
      if (!closed && current === generation)
        retry(cause instanceof Error ? cause.message : "事件连接失败");
    }
  }
  void open();
  return () => {
    closed = true;
    generation++;
    if (timer) clearTimeout(timer);
    stop();
  };
}
