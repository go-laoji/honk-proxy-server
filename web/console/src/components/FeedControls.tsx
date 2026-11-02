import { Pause, Play, RefreshCw, Radio } from "lucide-react";
import { feedStates, type FeedState } from "../tasks";
export default function FeedControls({
  state,
  automatic,
  toggle,
  reconnect,
}: {
  state: FeedState;
  automatic: boolean;
  toggle: () => void;
  reconnect: () => void;
}) {
  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <p
          className="muted flex items-center gap-2 text-xs"
          role="status"
          aria-label="事件订阅状态"
        >
          <Radio size={15} />
          {feedStates[state.state]}
          {state.attempt ? `（第 ${state.attempt} 次）` : ""}
        </p>
        <div className="flex flex-wrap gap-2">
          <button className="btn" onClick={toggle}>
            {automatic ? <Pause size={15} /> : <Play size={15} />}
            {automatic ? "暂停实时订阅" : "恢复实时订阅"}
          </button>
          <button className="btn" onClick={reconnect}>
            <RefreshCw size={15} />
            补取 / 重新连接
          </button>
        </div>
      </div>
      {state.message && (
        <p
          role={state.state === "error" ? "alert" : undefined}
          className="break-words text-xs leading-6 text-amber-700 dark:text-amber-400"
        >
          {state.message}。数据保留在最后成功时刻。
        </p>
      )}
    </div>
  );
}
