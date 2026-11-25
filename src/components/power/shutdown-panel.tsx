import { useState } from "react";

import { AlarmClock, Power, Timer } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useShutdownState } from "@/hooks/use-shutdown-state";
import { ipc } from "@/lib/ipc";

/** 与 Rust 侧 `power::MIN_MINUTES` / `MAX_MINUTES` 保持一致。 */
const MIN_MINUTES = 1;
const MAX_MINUTES = 24 * 60;

/** 常用时长，点一下就开始倒计时。 */
const PRESETS = [
  { minutes: 15, label: "15 分钟" },
  { minutes: 30, label: "30 分钟" },
  { minutes: 60, label: "1 小时" },
  { minutes: 120, label: "2 小时" },
];

/** 剩余秒数 → `05:30` / `1:05:30`。 */
function formatRemaining(totalSeconds: number): string {
  const seconds = Math.max(0, Math.floor(totalSeconds));
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const rest = seconds % 60;
  const mm = String(minutes).padStart(2, "0");
  const ss = String(rest).padStart(2, "0");
  return hours > 0 ? `${hours}:${mm}:${ss}` : `${mm}:${ss}`;
}

/** 截止时刻 → `今天 23:30` / `明天 01:00` / `3月2日 09:00`。 */
function formatDeadline(secs: number): string {
  const date = new Date(secs * 1000);
  const clock = `${String(date.getHours()).padStart(2, "0")}:${String(
    date.getMinutes(),
  ).padStart(2, "0")}`;

  const today = new Date();
  const startOfToday = new Date(
    today.getFullYear(),
    today.getMonth(),
    today.getDate(),
  ).getTime();
  const startOfTarget = new Date(
    date.getFullYear(),
    date.getMonth(),
    date.getDate(),
  ).getTime();
  const days = Math.round((startOfTarget - startOfToday) / 86_400_000);

  if (days === 0) return `今天 ${clock}`;
  if (days === 1) return `明天 ${clock}`;
  return `${date.getMonth() + 1}月${date.getDate()}日 ${clock}`;
}

/**
 * 主窗口「定时关机」页：预设时长 / 自定义分钟数 → 系统倒计时，可随时取消。
 *
 * 倒计时由 Windows 的 `shutdown /s /t` 维护（见 `src-tauri/src/power.rs`），
 * 所以关掉 Prism3 也不会取消；这里只负责开始、展示剩余时间与取消。
 */
export function ShutdownPanel() {
  const { state, remainingSecs } = useShutdownState();
  const [minutes, setMinutes] = useState("30");
  const [busy, setBusy] = useState(false);

  const start = async (value: number) => {
    if (!Number.isFinite(value)) {
      toast.error("请输入有效的分钟数");
      return;
    }
    const clamped = Math.min(MAX_MINUTES, Math.max(MIN_MINUTES, Math.round(value)));
    setBusy(true);
    try {
      await ipc.scheduleShutdown(clamped);
      toast.success("已开始倒计时");
    } catch (cause) {
      toast.error(`设置失败：${String(cause)}`);
    } finally {
      setBusy(false);
    }
  };

  const startCustom = () => {
    const parsed = Number.parseInt(minutes, 10);
    if (!Number.isFinite(parsed)) {
      toast.error("请输入有效的分钟数");
      return;
    }
    void start(parsed);
  };

  const cancel = async () => {
    setBusy(true);
    try {
      await ipc.cancelShutdown();
      toast.success("已取消定时关机");
    } catch (cause) {
      toast.error(`取消失败：${String(cause)}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex flex-col gap-4">
      <Card>
        <CardHeader>
          <CardTitle>定时关机</CardTitle>
          <CardDescription>
            到点后由 Windows 关闭计算机；请在倒计时结束前保存好工作，
            未保存的程序可能被系统阻止关闭。
          </CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col gap-5">
          {state.active ? (
            <div className="flex flex-col gap-4 rounded-xl bg-brand-soft/60 px-4 py-5">
              <div className="flex items-center gap-3">
                <span className="flex size-10 shrink-0 items-center justify-center rounded-full bg-brand text-primary-foreground">
                  <AlarmClock className="size-5" aria-hidden />
                </span>
                <div className="flex min-w-0 flex-col gap-1">
                  <span className="text-xs text-muted-foreground">距离关机还有</span>
                  <span className="font-mono text-3xl font-semibold leading-none tabular-nums text-foreground">
                    {remainingSecs != null ? formatRemaining(remainingSecs) : "--:--"}
                  </span>
                </div>
              </div>

              {state.deadlineSecs != null && (
                <p className="text-xs text-muted-foreground">
                  将于 {formatDeadline(state.deadlineSecs)} 关机
                </p>
              )}

              <div>
                <Button variant="outline" disabled={busy} onClick={() => void cancel()}>
                  <Power data-icon="inline-start" />
                  取消定时关机
                </Button>
              </div>
            </div>
          ) : (
            <>
              <div className="flex flex-col gap-2">
                <Label>快速设置</Label>
                <div className="grid grid-cols-4 gap-2">
                  {PRESETS.map((preset) => (
                    <Button
                      key={preset.minutes}
                      variant="outline"
                      disabled={busy}
                      onClick={() => void start(preset.minutes)}
                    >
                      {preset.label}
                    </Button>
                  ))}
                </div>
              </div>

              <div className="flex flex-col gap-2">
                <Label htmlFor="shutdown-minutes">自定义分钟数</Label>
                <div className="flex flex-wrap items-center gap-2">
                  <Input
                    id="shutdown-minutes"
                    type="number"
                    min={MIN_MINUTES}
                    max={MAX_MINUTES}
                    value={minutes}
                    onChange={(event) => setMinutes(event.target.value)}
                    onKeyDown={(event) => {
                      if (event.key === "Enter") {
                        event.preventDefault();
                        startCustom();
                      }
                    }}
                    className="h-8 w-28 text-xs"
                  />
                  <span className="text-xs text-muted-foreground">
                    分钟（1 分钟 – 24 小时）
                  </span>
                  <Button disabled={busy} onClick={startCustom}>
                    <Timer data-icon="inline-start" />
                    开始倒计时
                  </Button>
                </div>
              </div>
            </>
          )}
        </CardContent>
      </Card>

      <p className="px-1 text-xs text-muted-foreground">
        倒计时由系统维护，关掉 Prism3 也不会取消；要中止请在倒计时结束前点「取消定时关机」。
      </p>
    </div>
  );
}
