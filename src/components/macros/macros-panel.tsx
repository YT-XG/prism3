import { getCurrentWindow } from "@tauri-apps/api/window";
import { useCallback, useEffect, useRef, useState } from "react";

import {
  CirclePlay,
  CircleStop,
  LoaderCircle,
  Plus,
  Radio,
  Save,
  Trash2,
  Workflow,
} from "lucide-react";
import { toast } from "sonner";

import { MacroStepsEditor, actionLabel } from "@/components/macros/macro-step-list";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ipc, type MacroEntry, type MacroRecording, type MacroState, type MacroStep } from "@/lib/ipc";
import { cn } from "@/lib/utils";

function createMacroId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return crypto.randomUUID();
  }
  return `macro-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

function nowSecs(): number {
  return Math.floor(Date.now() / 1000);
}

/** 录制草稿的默认名字：`录制 11-04 14:32`。 */
function recordingName(): string {
  const now = new Date();
  const pad = (value: number) => value.toString().padStart(2, "0");
  return `录制 ${pad(now.getMonth() + 1)}-${pad(now.getDate())} ${pad(now.getHours())}:${pad(now.getMinutes())}`;
}

const INITIAL_STATE: MacroState = {
  recording: false,
  recordedSteps: 0,
  playing: false,
  pendingRecording: false,
  originLabel: null,
  recordingCompat: false,
};

/** 从按钮停止录制时，把「点停止按钮」产生的那一组鼠标按下 / 抬起步骤丢掉。 */
function stripTrailingStopClick(steps: MacroStep[]): MacroStep[] {
  const last = steps[steps.length - 1];
  if (last?.action.type === "mouseClick") {
    // 兼容旧的录制数据（旧版本把一次点击存成单步）。
    return steps.slice(0, -1);
  }
  if (last?.action.type !== "mouseUp") return steps;

  const previous = steps[steps.length - 2];
  if (
    previous?.action.type === "mouseDown" &&
    previous.action.button === last.action.button
  ) {
    return steps.slice(0, -2);
  }
  return steps;
}

/**
 * 「宏」页：录制全局鼠标点击 / 键盘操作，回放录好的步骤，也可以拖拽编排。
 *
 * 录制中按 F9 结束；停止按钮的点击本身也会被录进去，因此从按钮停止时会把最后
 * 那组鼠标按下 / 抬起丢掉（从 F9 结束则保留全部）。播放中按任意键立即停止。
 */
export function MacrosPanel() {
  const windowLabel = getCurrentWindow().label;
  const [macros, setMacros] = useState<MacroEntry[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [dirty, setDirty] = useState(false);
  const [saving, setSaving] = useState(false);
  const [busy, setBusy] = useState(false);
  const [pendingDelete, setPendingDelete] = useState(false);
  /** 回放前的倒计时（秒）；给用户时间切到目标窗口。 */
  const [countdown, setCountdown] = useState<number | null>(null);
  const [runner, setRunner] = useState<MacroState>(INITIAL_STATE);
  /** 本地是否认为正在录制（轮询发现后端已停时据此触发「取回录制」）。 */
  const recordingRef = useRef(false);
  /** 防止停止 / 轮询重复取回同一段录制。 */
  const finalizingRef = useRef(false);
  /** 记录上一轮播放状态，用于播放结束时提示。 */
  const wasPlayingRef = useRef(false);
  /** 倒计时结束后要回放的步骤快照。 */
  const pendingStepsRef = useRef<MacroStep[] | null>(null);

  const selected = macros.find((entry) => entry.id === selectedId) ?? macros[0] ?? null;
  const locked = runner.recording || runner.playing || countdown !== null;

  /** 把一次录制结果作为未保存草稿加进列表。 */
  const applyRecording = useCallback((result: MacroRecording) => {
    if (result.steps.length === 0) {
      toast.info("没有录到操作");
      return;
    }
    const now = nowSecs();
    const entry: MacroEntry = {
      id: createMacroId(),
      name: recordingName(),
      steps: result.steps,
      createdAt: now,
      updatedAt: now,
    };
    setMacros((current) => [...current, entry]);
    setSelectedId(entry.id);
    setPendingDelete(false);
    setDirty(true);
    toast.success(`录制完成：${result.steps.length} 步`, {
      description:
        result.skipped > 0
          ? `跳过 ${result.skipped} 个无法识别的按键；已作为未保存草稿加入，记得保存。`
          : result.compat
            ? "钩子被系统或游戏拦截，本次已用轮询兼容模式录制；已作为未保存草稿加入，记得保存。"
            : "已作为未保存草稿加入列表，记得点保存。",
    });
  }, []);

  /** 结束后端录制并把步骤取回来。`dropTrailingClick` 用于丢掉「点停止按钮」这一下。 */
  const finalizeRecording = useCallback(
    async (dropTrailingClick: boolean) => {
      if (finalizingRef.current) return;
      finalizingRef.current = true;
      try {
        const result = await ipc.stopMacroRecording();
        const steps = dropTrailingClick
          ? stripTrailingStopClick(result.steps)
          : result.steps;
        applyRecording({ ...result, steps });
      } catch (cause) {
        toast.error(`取回录制失败：${String(cause)}`);
      } finally {
        finalizingRef.current = false;
        const next = await ipc.getMacroState().catch(() => null);
        if (next) setRunner(next);
      }
    },
    [applyRecording],
  );

  // 首次挂载：读宏列表；如果上次在别的页面按了 F9，这里把遗留的录制取回来。
  useEffect(() => {
    let alive = true;
    void (async () => {
      try {
        const [list, state] = await Promise.all([ipc.getMacros(), ipc.getMacroState()]);
        if (!alive) return;
        setMacros(list);
        setSelectedId(list[0]?.id ?? null);
        setRunner(state);
        recordingRef.current = state.recording;
        wasPlayingRef.current = state.playing;
        if (
          !state.recording &&
          state.pendingRecording &&
          (!state.originLabel || state.originLabel === windowLabel)
        ) {
          await finalizeRecording(false);
        }
      } catch (cause) {
        if (alive) toast.error(`读取宏列表失败：${String(cause)}`);
      } finally {
        if (alive) setLoading(false);
      }
    })();
    return () => {
      alive = false;
    };
  }, [finalizeRecording, windowLabel]);

  // 录制 / 回放期间轮询状态：更新步数、检测 F9 结束、看到播放完成。
  const active = runner.recording || runner.playing;
  useEffect(() => {
    if (!active) return;
    let cancelled = false;
    const poll = async () => {
      try {
        const next = await ipc.getMacroState();
        if (cancelled) return;
        if (wasPlayingRef.current && !next.playing) {
          toast.info("宏回放已结束");
        }
        wasPlayingRef.current = next.playing;

        const ownsOrigin = !next.originLabel || next.originLabel === windowLabel;
        if (!next.recording && (recordingRef.current || next.pendingRecording) && ownsOrigin) {
          recordingRef.current = false;
          await finalizeRecording(false);
          return;
        }
        setRunner(next);
      } catch {
        // 轮询失败不打扰用户：下一次继续试；真正的错误由操作命令自己提示。
      }
    };
    void poll();
    const timer = window.setInterval(() => void poll(), 450);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [active, finalizeRecording, windowLabel]);

  // 主窗口与组合窗口都可能同时挂载宏页：重新获得焦点时（且没有未保存修改）
  // 拉一次最新列表，避免另一个窗口刚存下的宏在本窗口显示成旧数据。
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void getCurrentWindow()
      .onFocusChanged(({ payload }) => {
        if (!payload || disposed) return;
        if (dirty || runner.recording || runner.playing || countdown !== null) return;
        void ipc
          .getMacros()
          .then((list) => {
            if (disposed) return;
            setMacros(list);
            setSelectedId((current) =>
              current && list.some((entry) => entry.id === current)
                ? current
                : (list[0]?.id ?? null),
            );
          })
          .catch(() => {
            // 刷新失败保留当前列表，不打扰用户。
          });
      })
      .then((off) => {
        if (disposed) off();
        else unlisten = off;
      });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [dirty, runner.recording, runner.playing, countdown]);

  const startRecording = async () => {
    if (runner.recording || runner.playing || busy || countdown !== null) return;
    setBusy(true);
    try {
      await ipc.startMacroRecording();
      recordingRef.current = true;
      setRunner(await ipc.getMacroState());
      toast.info("录制中", {
        description: "完成操作后按 F9 结束；点上面的停止按钮也能结束（那一击会被删掉）。",
      });
    } catch (cause) {
      recordingRef.current = false;
      toast.error(`开始录制失败：${String(cause)}`);
    } finally {
      setBusy(false);
    }
  };

  const stopRecording = async () => {
    recordingRef.current = false;
    await finalizeRecording(true);
  };

  /** 倒计时结束后的真正回放。 */
  async function beginPlayback(steps: MacroStep[]) {
    setBusy(true);
    try {
      await ipc.playMacro(steps);
      const next = await ipc.getMacroState();
      setRunner(next);
      wasPlayingRef.current = next.playing;
      if (next.playing) {
        toast.info("开始回放", { description: "按任意键立即停止。" });
      }
    } catch (cause) {
      toast.error(`回放失败：${String(cause)}`);
    } finally {
      setBusy(false);
    }
  }

  const play = () => {
    if (!selected || selected.steps.length === 0) {
      toast.info("这个宏还没有步骤，先录制或拖入几个动作");
      return;
    }
    if (runner.recording || runner.playing || busy || countdown !== null) return;
    // 回放会直接操作当前焦点窗口；先留 3 秒给用户切过去。
    pendingStepsRef.current = selected.steps;
    setCountdown(3);
  };

  // 回放倒计时：3 → 2 → 1 → 0 后执行；组件卸载时定时器会被清掉。
  useEffect(() => {
    if (countdown === null) return;
    if (countdown === 0) {
      const steps = pendingStepsRef.current;
      pendingStepsRef.current = null;
      setCountdown(null);
      if (steps) void beginPlayback(steps);
      return;
    }
    const timer = window.setTimeout(() => {
      setCountdown((current) => (current === null ? null : current - 1));
    }, 1000);
    return () => window.clearTimeout(timer);
  }, [countdown]);

  const cancelCountdown = () => {
    pendingStepsRef.current = null;
    setCountdown(null);
  };

  const stopPlayback = async () => {
    try {
      await ipc.stopMacroPlayback();
      setRunner(await ipc.getMacroState());
    } catch (cause) {
      toast.error(`停止回放失败：${String(cause)}`);
    }
  };

  const addMacro = () => {
    const now = nowSecs();
    const entry: MacroEntry = {
      id: createMacroId(),
      name: `新宏 ${macros.length + 1}`,
      steps: [],
      createdAt: now,
      updatedAt: now,
    };
    setMacros((current) => [...current, entry]);
    setSelectedId(entry.id);
    setPendingDelete(false);
    setDirty(true);
  };

  const updateSelected = (patch: Partial<Pick<MacroEntry, "name" | "steps">>) => {
    if (!selected) return;
    setMacros((current) =>
      current.map((entry) => (entry.id === selected.id ? { ...entry, ...patch } : entry)),
    );
    setDirty(true);
  };

  const deleteSelected = () => {
    if (!selected) return;
    const next = macros.filter((entry) => entry.id !== selected.id);
    setMacros(next);
    setSelectedId(next[0]?.id ?? null);
    setPendingDelete(false);
    setDirty(true);
  };

  const save = async () => {
    if (saving) return;
    setSaving(true);
    try {
      await ipc.saveMacros(macros);
      const saved = await ipc.getMacros();
      setMacros(saved);
      setSelectedId((current) =>
        current && saved.some((entry) => entry.id === current) ? current : (saved[0]?.id ?? null),
      );
      setDirty(false);
      toast.success("宏已保存");
    } catch (cause) {
      toast.error(`保存失败：${String(cause)}`);
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="flex h-full min-h-0 w-full flex-col gap-4">
      <div className="flex shrink-0 flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <h1 className="font-heading text-lg font-medium">宏</h1>
          <p className="text-sm text-muted-foreground">
            录制鼠标与键盘的按下 / 抬起，按住时长和键鼠并发都会保留；也可以把动作积木拖进列表编排。
            开始录制后本窗口会自动隐藏，按 F9 结束时自动回来。
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-1.5">
          <Button size="sm" variant="outline" disabled={locked} onClick={addMacro}>
            <Plus data-icon="inline-start" />
            新建宏
          </Button>
          {runner.recording ? (
            <Button size="sm" variant="destructive" onClick={() => void stopRecording()}>
              <CircleStop data-icon="inline-start" />
              停止录制 · {runner.recordedSteps} 步
            </Button>
          ) : (
            <Button
              size="sm"
              disabled={runner.playing || busy || countdown !== null}
              title="开始录制后本窗口会隐藏；按 F9 结束并自动回来"
              onClick={() => void startRecording()}
            >
              <Radio data-icon="inline-start" />
              开始录制
            </Button>
          )}
        </div>
      </div>

      {runner.recording && (
        <div className="flex shrink-0 items-center gap-2 rounded-lg bg-red-500/10 px-3 py-2 text-xs text-red-600 dark:text-red-400">
          <span className="relative flex size-2">
            <span className="absolute inset-0 animate-ping rounded-full bg-red-500/60" />
            <span className="size-2 rounded-full bg-red-500" />
          </span>
          <span
            title={
              runner.recordingCompat
                ? "游戏或反作弊拦截了系统低级钩子，已自动改用轮询采集；极快的点按仍可能漏记。"
                : undefined
            }
          >
            录制中 · 已记录 {runner.recordedSteps} 步 · 按 F9 结束
            {runner.recordingCompat ? " · 兼容模式（轮询）" : ""}
          </span>
          <span className="text-muted-foreground">
            停止按钮的点击也会被记入，结束后会自动删掉最后一下。
          </span>
        </div>
      )}

      {runner.playing && (
        <div className="flex shrink-0 items-center gap-2 rounded-lg bg-amber-500/10 px-3 py-2 text-xs text-amber-700 dark:text-amber-400">
          <span className="relative flex size-2">
            <span className="absolute inset-0 animate-ping rounded-full bg-amber-500/60" />
            <span className="size-2 rounded-full bg-amber-500" />
          </span>
          <span>回放中 · 按任意键立即停止</span>
          <span className="text-muted-foreground">鼠标正在被宏接管，停止按钮可能点不到。</span>
        </div>
      )}

      {countdown !== null && !runner.playing && (
        <div className="flex shrink-0 items-center gap-2 rounded-lg bg-brand-soft px-3 py-2 text-xs text-brand">
          <span className="relative flex size-2">
            <span className="absolute inset-0 animate-ping rounded-full bg-brand/60" />
            <span className="size-2 rounded-full bg-brand" />
          </span>
          <span>{countdown} 秒后开始回放 · 请切到目标窗口</span>
          <span className="text-muted-foreground">倒计时结束前可按下方取消</span>
          <Button variant="ghost" size="xs" className="ml-auto" onClick={cancelCountdown}>
            取消
          </Button>
        </div>
      )}

      {loading ? (
        <div className="flex flex-1 items-center justify-center gap-2 text-sm text-muted-foreground">
          <LoaderCircle className="size-4 animate-spin" aria-hidden="true" />
          读取宏列表…
        </div>
      ) : macros.length === 0 ? (
        <div className="flex flex-1 flex-col items-center justify-center gap-3 rounded-xl border border-dashed border-border/70 px-6 py-14 text-center">
          <span className="flex size-10 items-center justify-center rounded-xl bg-brand-soft text-brand">
            <Workflow className="size-5" aria-hidden="true" />
          </span>
          <div>
            <p className="text-sm font-medium">还没有宏</p>
            <p className="mt-1 text-xs text-muted-foreground">
              录一段每天都要做的操作，或者新建空宏后把动作积木拖进去。
            </p>
          </div>
          <div className="flex items-center gap-1.5">
            <Button size="sm" disabled={locked || busy} onClick={() => void startRecording()}>
              <Radio data-icon="inline-start" />
              开始录制
            </Button>
            <Button size="sm" variant="outline" disabled={locked} onClick={addMacro}>
              <Plus data-icon="inline-start" />
              新建空宏
            </Button>
          </div>
        </div>
      ) : (
        <div className="flex min-h-0 flex-1 gap-4">
          {/* 左列：宏列表 */}
          <aside className="flex w-60 shrink-0 flex-col gap-1 overflow-y-auto rounded-xl p-1.5 ring-1 ring-[color:var(--frost-border)] [background-image:var(--frost-surface)]">
            {macros.map((entry) => {
              const activeEntry = entry.id === selected?.id;
              return (
                <button
                  key={entry.id}
                  type="button"
                  disabled={runner.playing || countdown !== null}
                  onClick={() => {
                    setSelectedId(entry.id);
                    setPendingDelete(false);
                  }}
                  className={cn(
                    "flex w-full shrink-0 flex-col gap-0.5 rounded-lg px-2.5 py-2 text-left transition-colors",
                    activeEntry ? "bg-brand-soft" : "hover:bg-muted/50",
                    (runner.playing || countdown !== null) && "pointer-events-none opacity-60",
                  )}
                >
                  <span className="flex min-w-0 items-center gap-1.5">
                    <Workflow
                      className={cn(
                        "size-3.5 shrink-0",
                        activeEntry ? "text-brand" : "text-muted-foreground",
                      )}
                      aria-hidden="true"
                    />
                    <span className="min-w-0 flex-1 truncate text-xs font-medium">
                      {entry.name}
                    </span>
                    <span className="shrink-0 rounded bg-muted/60 px-1 text-[0.6rem] text-muted-foreground tabular-nums">
                      {entry.steps.length}
                    </span>
                  </span>
                  <span className="truncate pl-5 text-[0.65rem] text-muted-foreground">
                    {entry.steps.length > 0 ? actionLabel(entry.steps[0].action) : "空宏"}
                  </span>
                </button>
              );
            })}
            {dirty && (
              <p className="mt-auto px-1.5 pt-1 text-[0.65rem] text-amber-600 dark:text-amber-400">
                有未保存的修改
              </p>
            )}
          </aside>

          {/* 右列：选中的宏 */}
          <section className="flex min-w-0 flex-1 flex-col gap-3">
            {selected && (
              <>
                <div className="flex shrink-0 flex-wrap items-center gap-2 rounded-xl px-3 py-2.5 ring-1 ring-[color:var(--frost-border)] [background-image:var(--frost-surface)]">
                  <span className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-brand-soft text-brand">
                    <Workflow className="size-4" aria-hidden="true" />
                  </span>
                  <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                    <div className="flex min-w-0 items-center gap-1.5">
                      <Input
                        value={selected.name}
                        disabled={locked}
                        aria-label="宏名称"
                        spellCheck={false}
                        className="h-7 min-w-0 flex-1 border-transparent bg-transparent px-0 text-sm font-medium focus-visible:border-transparent focus-visible:ring-0"
                        onChange={(event) => updateSelected({ name: event.target.value })}
                      />
                      <span
                        className={cn(
                          "shrink-0 rounded px-1.5 py-0.5 text-[0.65rem]",
                          dirty
                            ? "bg-amber-500/15 text-amber-700 dark:text-amber-400"
                            : "bg-muted/40 text-muted-foreground",
                        )}
                      >
                        {dirty ? "未保存" : "已保存"}
                      </span>
                    </div>
                    <p className="text-xs text-muted-foreground">
                      {selected.steps.length} 步 · 回放使用当前编辑中的步骤，不强制先保存
                    </p>
                  </div>

                  <Button
                    variant="outline"
                    size="xs"
                    disabled={!dirty || saving || locked}
                    onClick={() => void save()}
                  >
                    {saving ? (
                      <LoaderCircle data-icon="inline-start" className="animate-spin" />
                    ) : (
                      <Save data-icon="inline-start" />
                    )}
                    {saving ? "保存中…" : "保存"}
                  </Button>

                  {runner.playing ? (
                    <Button variant="destructive" size="xs" onClick={() => void stopPlayback()}>
                      <CircleStop data-icon="inline-start" />
                      停止回放
                    </Button>
                  ) : (
                    <Button
                      variant="secondary"
                      size="xs"
                      disabled={runner.recording || busy || countdown !== null}
                      onClick={() => void play()}
                    >
                      <CirclePlay data-icon="inline-start" />
                      回放
                    </Button>
                  )}

                  {pendingDelete ? (
                    <span className="flex shrink-0 items-center gap-1">
                      <Button variant="destructive" size="xs" onClick={deleteSelected}>
                        确认删除
                      </Button>
                      <Button variant="ghost" size="xs" onClick={() => setPendingDelete(false)}>
                        取消
                      </Button>
                    </span>
                  ) : (
                    <Button
                      variant="ghost"
                      size="icon-xs"
                      disabled={locked}
                      aria-label={`删除 ${selected.name}`}
                      title="删除"
                      onClick={() => setPendingDelete(true)}
                    >
                      <Trash2 />
                    </Button>
                  )}
                </div>

                <div className="flex min-h-0 flex-1 flex-col gap-2.5 rounded-xl p-3 ring-1 ring-[color:var(--frost-border)] [background-image:var(--frost-surface)]">
                  <div className="flex shrink-0 items-center justify-between gap-2">
                    <p className="text-xs font-medium">步骤</p>
                    <p className="text-[0.65rem] text-muted-foreground">
                      回放会接管键鼠；停止键为任意真实按键
                    </p>
                  </div>
                  <MacroStepsEditor
                    steps={selected.steps}
                    disabled={locked}
                    onChange={(steps) => updateSelected({ steps })}
                  />
                </div>
              </>
            )}
          </section>
        </div>
      )}
    </div>
  );
}
