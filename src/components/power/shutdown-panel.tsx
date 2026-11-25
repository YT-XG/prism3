import { useEffect, useState } from "react";

import { AlarmClock, CalendarDays, Power, Timer } from "lucide-react";
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
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useShutdownState } from "@/hooks/use-shutdown-state";
import { ipc, type ShutdownMode } from "@/lib/ipc";
import { cn } from "@/lib/utils";

/** 与 Rust 侧 `power::MIN_MINUTES` / `MAX_MINUTES` 保持一致。 */
const MIN_MINUTES = 1;
const MAX_MINUTES = 24 * 60;

/** 常用倒计时时长，点一下就开始。 */
const PRESETS = [
  { minutes: 15, label: "15 分钟" },
  { minutes: 30, label: "30 分钟" },
  { minutes: 60, label: "1 小时" },
  { minutes: 120, label: "2 小时" },
];

const WEEKDAYS = [
  { value: 0, label: "日" },
  { value: 1, label: "一" },
  { value: 2, label: "二" },
  { value: 3, label: "三" },
  { value: 4, label: "四" },
  { value: 5, label: "五" },
  { value: 6, label: "六" },
];

const EVERY_DAY = WEEKDAYS.map((day) => day.value);
const WORKDAYS = [1, 2, 3, 4, 5];
const WEEKEND = [0, 6];

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

/** 星期数组 → `每天` / `工作日` / `周末` / `周一、周三`。 */
function formatDays(days: number[]): string {
  if (days.length === 0 || days.length === 7) return "每天";
  const sorted = [...days].sort((a, b) => a - b);
  if (sorted.join(",") === WORKDAYS.join(",")) return "工作日";
  if (sorted.join(",") === WEEKEND.join(",")) return "周末";
  return `周${sorted.map((day) => WEEKDAYS[day]?.label ?? "").join("、")}`;
}

/**
 * 主窗口「定时关机」页：倒计时 / 每天定时（可选星期）。
 *
 * 倒计时交给 Windows 的 `shutdown /s /t`；每天定时由 Prism3 的调度线程守着，
 * 因此需要应用保持运行（关闭主窗口会收进托盘）。
 */
export function ShutdownPanel() {
  const { state, remainingSecs } = useShutdownState();
  const [mode, setMode] = useState<ShutdownMode>("daily");
  const [minutes, setMinutes] = useState("30");
  const [time, setTime] = useState("23:00");
  /** 选中的星期；空数组 = 每天（与后端 `weekdays` 的口径一致）。 */
  const [days, setDays] = useState<number[]>([]);
  const [busy, setBusy] = useState(false);

  // 有正在执行的计划时，页签跟随真实模式；空闲时保留用户当前选择。
  useEffect(() => {
    if (state.mode) setMode(state.mode);
    else if (state.dailyTime) setMode("daily");
  }, [state.mode, state.dailyTime]);

  // 每天定时的时间 / 星期以服务端状态为准（重新打开页面、其它入口改过都能对齐）。
  useEffect(() => {
    if (state.dailyTime) setTime(state.dailyTime);
    setDays(state.weekdays.length === 0 ? [] : [...state.weekdays].sort((a, b) => a - b));
  }, [state.dailyTime, state.weekdays]);

  const startCountdown = async (value: number) => {
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

  const startCustomCountdown = () => {
    const parsed = Number.parseInt(minutes, 10);
    if (!Number.isFinite(parsed)) {
      toast.error("请输入有效的分钟数");
      return;
    }
    void startCountdown(parsed);
  };

  const saveDaily = async () => {
    if (!/^\d{2}:\d{2}$/.test(time)) {
      toast.error("请选择关机时间");
      return;
    }
    setBusy(true);
    try {
      await ipc.setDailyShutdown(time, days.length === 7 ? [] : days);
      toast.success(`已设置 ${formatDays(days)} ${time} 关机`);
    } catch (cause) {
      toast.error(`设置失败：${String(cause)}`);
    } finally {
      setBusy(false);
    }
  };

  const cancel = async () => {
    setBusy(true);
    try {
      await ipc.cancelShutdown();
      toast.success("已取消关机计划");
    } catch (cause) {
      toast.error(`取消失败：${String(cause)}`);
    } finally {
      setBusy(false);
    }
  };

  /** 星期多选：空数组代表每天；不允许一个都不选。 */
  const toggleDay = (day: number) => {
    const selected = days.length === 0 ? EVERY_DAY : days;
    const next = selected.includes(day)
      ? selected.filter((item) => item !== day)
      : [...selected, day].sort((a, b) => a - b);
    if (next.length === 0) return;
    setDays(next.length === 7 ? [] : next);
  };

  const isDaily = state.mode === "daily";
  const selectedDays = days.length === 0 ? EVERY_DAY : days;

  return (
    <div className="flex flex-col gap-4">
      <Card>
        <CardHeader>
          <CardTitle>定时关机</CardTitle>
          <CardDescription>
            到点后由 Windows 关闭计算机；请在关机前保存好工作，未保存的程序可能被系统阻止关闭。
          </CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col gap-5">
          {state.active ? (
            <div className="flex flex-col gap-4 rounded-xl bg-brand-soft/60 px-4 py-5">
              <div className="flex items-center gap-3">
                <span className="flex size-10 shrink-0 items-center justify-center rounded-full bg-brand text-primary-foreground">
                  {isDaily ? (
                    <CalendarDays className="size-5" aria-hidden />
                  ) : (
                    <AlarmClock className="size-5" aria-hidden />
                  )}
                </span>
                <div className="flex min-w-0 flex-col gap-1">
                  <span className="text-xs text-muted-foreground">
                    {isDaily
                      ? `${formatDays(state.weekdays)} ${state.dailyTime ?? ""} 关机 · 距离下一次`
                      : "倒计时关机 · 距离关机还有"}
                  </span>
                  <span className="font-mono text-3xl font-semibold leading-none tabular-nums text-foreground">
                    {remainingSecs != null ? formatRemaining(remainingSecs) : "--:--"}
                  </span>
                </div>
              </div>

              {state.deadlineSecs != null && (
                <p className="text-xs text-muted-foreground">
                  将于 {formatDeadline(state.deadlineSecs)} 关机
                  {isDaily ? "；关机后需等 Prism3 下次启动（默认开机自启）继续排期" : ""}
                </p>
              )}

              <div>
                <Button variant="outline" disabled={busy} onClick={() => void cancel()}>
                  <Power data-icon="inline-start" />
                  取消关机计划
                </Button>
              </div>
            </div>
          ) : (
            <>
              <Tabs value={mode} onValueChange={(value) => setMode(value as ShutdownMode)}>
                <TabsList>
                  <TabsTrigger value="countdown">
                    <Timer />
                    倒计时
                  </TabsTrigger>
                  <TabsTrigger value="daily">
                    <CalendarDays />
                    每天定时
                  </TabsTrigger>
                </TabsList>
              </Tabs>

              {mode === "countdown" ? (
                <>
                  <div className="flex flex-col gap-2">
                    <Label>快速设置</Label>
                    <div className="grid grid-cols-4 gap-2">
                      {PRESETS.map((preset) => (
                        <Button
                          key={preset.minutes}
                          variant="outline"
                          disabled={busy}
                          onClick={() => void startCountdown(preset.minutes)}
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
                            startCustomCountdown();
                          }
                        }}
                        className="h-8 w-28 text-xs"
                      />
                      <span className="text-xs text-muted-foreground">
                        分钟（1 分钟 – 24 小时）
                      </span>
                      <Button disabled={busy} onClick={startCustomCountdown}>
                        <Timer data-icon="inline-start" />
                        开始倒计时
                      </Button>
                    </div>
                  </div>
                </>
              ) : (
                <>
                  <div className="flex flex-col gap-2">
                    <Label htmlFor="shutdown-time">关机时间</Label>
                    <div className="flex flex-wrap items-center gap-2">
                      <Input
                        id="shutdown-time"
                        type="time"
                        value={time}
                        onChange={(event) => setTime(event.target.value)}
                        className="h-8 w-32 text-xs"
                      />
                      <span className="text-xs text-muted-foreground">每天到这个时刻关机</span>
                    </div>
                  </div>

                  <div className="flex flex-col gap-2">
                    <div className="flex flex-wrap items-baseline justify-between gap-2">
                      <Label>重复</Label>
                      <div className="flex gap-1">
                        {[
                          { label: "每天", value: EVERY_DAY },
                          { label: "工作日", value: WORKDAYS },
                          { label: "周末", value: WEEKEND },
                        ].map((option) => {
                          const active =
                            option.value.join(",") === selectedDays.join(",");
                          return (
                            <button
                              key={option.label}
                              type="button"
                              aria-pressed={active}
                              onClick={() =>
                                setDays(option.value.length === 7 ? [] : option.value)
                              }
                              className={cn(
                                "rounded-md px-2 py-1 text-xs transition-colors",
                                active
                                  ? "bg-brand-soft font-medium text-foreground"
                                  : "text-muted-foreground hover:bg-muted hover:text-foreground",
                              )}
                            >
                              {option.label}
                            </button>
                          );
                        })}
                      </div>
                    </div>

                    <div className="flex flex-wrap gap-1.5">
                      {WEEKDAYS.map((day) => {
                        const active = selectedDays.includes(day.value);
                        return (
                          <button
                            key={day.value}
                            type="button"
                            aria-pressed={active}
                            aria-label={`周${day.label}`}
                            onClick={() => toggleDay(day.value)}
                            className={cn(
                              "size-8 rounded-full text-xs transition-colors",
                              active
                                ? "bg-brand text-primary-foreground"
                                : "bg-muted text-muted-foreground hover:bg-muted/70 hover:text-foreground",
                            )}
                          >
                            {day.label}
                          </button>
                        );
                      })}
                    </div>
                    <span className="text-xs text-muted-foreground">
                      当前：{formatDays(days)} {time || "--:--"} 关机
                    </span>
                  </div>

                  <div>
                    <Button disabled={busy} onClick={() => void saveDaily()}>
                      <CalendarDays data-icon="inline-start" />
                      保存定时
                    </Button>
                  </div>

                  <p className="text-xs text-muted-foreground">
                    每天定时依赖 Prism3 保持运行（关闭主窗口会收进托盘，开机自启默认开启）；
                    要中止请先回到本页点「取消关机计划」。
                  </p>
                </>
              )}
            </>
          )}
        </CardContent>
      </Card>

      <p className="px-1 text-xs text-muted-foreground">
        倒计时由系统维护，关掉 Prism3 也不会取消；每天定时由本应用调度，退出应用即停止排期。
      </p>
    </div>
  );
}
