import { useEffect, useRef, useState } from "react";

import { Copy, Eraser, Square, Terminal, Trash2, X } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import type { ProjectOutputLine, ProjectRun } from "@/lib/ipc";
import { cn } from "@/lib/utils";

interface ProjectConsoleProps {
  /** 当前要显示的运行记录。 */
  run: ProjectRun;
  lines: ProjectOutputLine[];
  /** 外层布局类：项目页里用 `flex-1` 让它吃掉右列剩余高度。 */
  className?: string;
  onClose: () => void;
  onStop: (runId: string) => void;
  onClearLog: (runId: string) => void;
  /** 删除记录（仅已结束的记录可选）。 */
  onClearRun: (runId: string) => void;
}

/** 距底部多少像素以内算「跟着滚动」，用户往上翻看历史时不打断。 */
const STICK_THRESHOLD = 32;

/** 运行状态徽标。 */
function statusOf(run: ProjectRun): { text: string; tone: string } {
  if (run.running) {
    return {
      text: run.stopping ? "正在停止…" : "运行中",
      tone: "bg-emerald-500/15 text-emerald-600 dark:text-emerald-400",
    };
  }
  if (run.stopping) {
    return { text: "已停止", tone: "bg-muted text-muted-foreground" };
  }
  if (run.exitCode === 0) {
    return { text: "已结束", tone: "bg-muted text-muted-foreground" };
  }
  return {
    text: run.exitCode === null ? "已结束" : `退出码 ${run.exitCode}`,
    tone: "bg-destructive/10 text-destructive",
  };
}

/**
 * 项目页内嵌的命令控制台：实时输出 + 运行状态 + 停止 / 清空 / 复制。
 *
 * 深色底固定不变（亮色主题下也像一块终端），滚动条默认贴底；
 * 用户往上翻看历史时暂停自动跟随，回到底部后恢复。
 */
export function ProjectConsole({
  run,
  lines,
  className,
  onClose,
  onStop,
  onClearLog,
  onClearRun,
}: ProjectConsoleProps) {
  const viewportRef = useRef<HTMLDivElement>(null);
  const [stick, setStick] = useState(true);
  const status = statusOf(run);

  useEffect(() => {
    if (!stick) return;
    const viewport = viewportRef.current;
    if (viewport) viewport.scrollTop = viewport.scrollHeight;
  }, [lines, stick]);

  const handleScroll = () => {
    const viewport = viewportRef.current;
    if (!viewport) return;
    const distance =
      viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight;
    setStick(distance <= STICK_THRESHOLD);
  };

  const copyAll = () => {
    const text = lines.map((line) => line.text).join("\n");
    void navigator.clipboard.writeText(text).then(
      () => toast.success("已复制全部输出"),
      (cause) => toast.error(`复制失败：${String(cause)}`),
    );
  };

  return (
    <div
      className={cn(
        "flex min-h-0 flex-col overflow-hidden rounded-xl ring-1 ring-[color:var(--frost-border)] [background-image:var(--frost-surface)]",
        className,
      )}
    >
      <div className="flex shrink-0 flex-wrap items-center gap-2 border-b border-border/60 px-3 py-2">
        <Terminal className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
        <div className="min-w-0 flex-1">
          <p className="truncate text-xs font-medium">
            {run.projectName}
            <span className="mx-1 text-muted-foreground">·</span>
            {run.commandName}
          </p>
          <p className="truncate text-[0.65rem] text-muted-foreground" title={run.workdir}>
            {run.workdir}
          </p>
        </div>

        <span
          className={cn(
            "shrink-0 rounded-full px-2 py-0.5 text-[0.65rem] font-medium tabular-nums",
            status.tone,
          )}
        >
          {status.text}
        </span>

        <div className="flex shrink-0 items-center gap-0.5">
          {run.running && (
            <Button
              variant="destructive"
              size="xs"
              title="停止并结束整棵进程树"
              onClick={() => onStop(run.runId)}
            >
              <Square data-icon="inline-start" />
              停止
            </Button>
          )}
          <Button variant="ghost" size="icon-xs" title="复制全部输出" onClick={copyAll}>
            <Copy />
          </Button>
          <Button
            variant="ghost"
            size="icon-xs"
            title="清空输出（后端缓冲一并清空）"
            onClick={() => onClearLog(run.runId)}
          >
            <Eraser />
          </Button>
          {!run.running && (
            <Button
              variant="ghost"
              size="icon-xs"
              title="删除这条运行记录"
              onClick={() => onClearRun(run.runId)}
            >
              <Trash2 />
            </Button>
          )}
          <Button variant="ghost" size="icon-xs" title="收起控制台" onClick={onClose}>
            <X />
          </Button>
        </div>
      </div>

      <div
        ref={viewportRef}
        onScroll={handleScroll}
        className="min-h-0 flex-1 overflow-auto bg-[#101116] px-3 py-2.5"
      >
        {lines.length === 0 ? (
          <p className="font-mono text-[11px] text-zinc-500">等待输出…</p>
        ) : (
          <pre className="font-mono text-[11px] leading-relaxed whitespace-pre-wrap break-all">
            {lines.map((line, index) => (
              <span
                key={index}
                className={cn(
                  line.stream === "stderr"
                    ? "text-red-300"
                    : line.stream === "system"
                      ? "text-zinc-500"
                      : "text-zinc-200",
                )}
              >
                {line.text}
                {"\n"}
              </span>
            ))}
          </pre>
        )}
      </div>
    </div>
  );
}
