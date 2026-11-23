import { useEffect, useRef, useState } from "react";

import { CircleAlert, CircleCheck, GripVertical, RefreshCw } from "lucide-react";
import { debug as logDebug } from "@tauri-apps/plugin-log";

import { ipc } from "@/lib/ipc";
import { cn } from "@/lib/utils";

/**
 * 悬浮窗里的一枚按键有五种状态：
 * `idle` 待按、`busy` 正在发锁屏指令、`done` 已发成功、`error` 失败、`offline` 没找到已授权的设备。
 */
type Phase = "idle" | "busy" | "done" | "error" | "offline";

/** 成功提示停留多久再回到「待按」（再久就挡住下一次点击的手感）。 */
const FLASH_MS = 1600;

/**
 * 手机「老板模式」悬浮窗（window label = `phone-boss`，由 `quick-panel.html` 承载）。
 *
 * 整块挂件只有**一个按键：息屏**。它的用途是「老板走过来」那一刻：
 * 镜像窗口可以最小化到托盘，这枚按钮始终置顶浮在所有窗口上面，按一下就把手机屏幕
 * 熄掉（走 `phone_lock`：先 `KEYCODE_SLEEP`，读不到屏幕状态时绝不补按电源键）。
 *
 * 三条刻意的设计：
 * 1. **窗口不抢焦点**（`focusable: false`，见 `commands.rs::build_phone_boss_window`），
 *    所以按下它不会把用户正在用的窗口顶到后面；按钮本身靠鼠标事件照常工作。
 * 2. 设备序列号**每次点击都现探一次**（`phone_status` 只读 adb devices，几十毫秒）：
 *    挂件是常驻的，中途拔插数据线、换手机都不该让它变成一个点不动的按钮。
 * 3. 退出老板模式不占一个按钮：**右键挂件**（或回手机页再点一次「老板模式」）。
 */
export function PhoneBossPanel() {
  const [phase, setPhase] = useState<Phase>("idle");
  const timerRef = useRef<number | undefined>(undefined);

  useEffect(() => {
    return () => {
      if (timerRef.current !== undefined) window.clearTimeout(timerRef.current);
    };
  }, []);

  // 挂件曾出现过「窗口可见、内容却全透明」的事故：建窗成功了，但 React 有没有挂上
  // 从界面完全看不出来。挂载即留一条日志，下次一眼就能区分「没渲染」和「渲染了但看不见」。
  useEffect(() => {
    void logDebug("老板模式挂件已挂载");
  }, []);

  /** 现探一次 adb，返回「已授权的那台」的序列号；探不到返回 null。 */
  const resolveSerial = async (): Promise<string | null> => {
    try {
      const status = await ipc.phoneStatus();
      const device = status.devices.find(
        (item) => item.serial === status.selected && item.authorized,
      );
      return device?.serial ?? null;
    } catch {
      return null;
    }
  };

  // 挂上就先探一次：没设备时按钮直接显示「未连接」，而不是让人点了才失败。
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const serial = await resolveSerial();
      if (!cancelled && !serial) setPhase("offline");
    })();
    return () => {
      cancelled = true;
    };
    // 只在挂载时探一次；之后每次点击都会重新探。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const schedule = (next: Phase) => {
    if (timerRef.current !== undefined) window.clearTimeout(timerRef.current);
    timerRef.current = window.setTimeout(() => {
      timerRef.current = undefined;
      setPhase(next);
    }, FLASH_MS);
  };

  const lock = async () => {
    if (phase === "busy") return;
    setPhase("busy");
    const serial = await resolveSerial();
    if (!serial) {
      setPhase("offline");
      return;
    }
    try {
      // 返回值（「已锁屏」这类说明）**刻意不展示**：挂件可能一直浮在桌面上，
      // 连 tooltip 里出现「已锁屏」都等于把用途写在脸上。成功/失败只用颜色与图标表达。
      await ipc.phoneLock(serial);
      setPhase("done");
      schedule("idle");
    } catch {
      setPhase("error");
      schedule("idle");
    }
  };

  const label =
    phase === "busy"
      ? "同步中…"
      : phase === "done"
        ? "已同步"
        : phase === "error"
          ? "重试"
          : phase === "offline"
            ? "未连接"
            : "同步";

  const Icon =
    phase === "done" ? CircleCheck : phase === "error" ? CircleAlert : RefreshCw;

  // 文案刻意保持中性：这块挂件可能长时间浮在桌面上，不该让旁人一眼看出它在动手机。
  // 连 tooltip 都只说「同步」——成功/失败只靠按钮颜色与图标表达，不写结果说明。
  const tip = "同步设备状态 · 拖动可移动位置 · 右键关闭";

  return (
    <div
      data-tauri-drag-region
      title={tip}
      onContextMenu={(event) => {
        // 右键 = 退出老板模式：悬浮窗上只留一个按键，出口放在这里。
        event.preventDefault();
        void ipc.closePhoneBoss().catch(() => {});
      }}
      // 外层只是透明留白（窗口比胶囊大一圈，留着拖动与点击余量）。
      className="flex h-full w-full items-center justify-center"
    >
      <div className="flex h-14 w-[136px] items-center gap-0.5 rounded-2xl bg-[#10141c]/95 p-1">
        {/* 拖动把手：整窗唯一的「非按键」区域，窗口不给焦点，只能靠它换位置。 */}
        <div
          data-tauri-drag-region
          className="flex h-full w-4 shrink-0 cursor-move items-center justify-center text-white/35"
        >
          <GripVertical className="size-3.5" />
        </div>
        <button
          type="button"
          onClick={() => void lock()}
          title={tip}
          aria-label={tip}
          className={cn(
            "flex h-full min-w-0 flex-1 items-center justify-center gap-1.5 rounded-xl text-[13px] font-medium text-white transition-[filter,background-color,transform] select-none",
            "active:scale-[0.985] active:brightness-95",
            phase === "idle" && "bg-[var(--brand)] hover:brightness-110",
            phase === "busy" && "animate-pulse bg-[var(--brand)]",
            phase === "done" && "bg-emerald-600/90",
            phase === "error" && "bg-red-600/90",
            phase === "offline" && "bg-white/10 text-white/60 hover:bg-white/15",
          )}
        >
          <Icon className="size-3.5" />
          {label}
        </button>
      </div>
    </div>
  );
}
