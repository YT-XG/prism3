import { useEffect, useMemo, useRef, useState, type MouseEvent } from "react";

import { CheckCircle2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useWorkLogs, useWorkProjects } from "@/hooks/use-work-logs";
import { ipc } from "@/lib/ipc";
import { WORK_KIND_LABELS } from "@/lib/work-kinds";
import { parseQuickLog } from "@/lib/work-parse";
import { endOfDay, formatClock, formatHours, startOfDay } from "@/lib/work-time";
import { cn } from "@/lib/utils";

interface QuickPanelWorkProps {
  /** 面板已展开且当前就是记录页。 */
  active: boolean;
  /** 是否已进入输入聚焦模式。 */
  focused: boolean;
  onRequestFocus: () => Promise<boolean>;
  onReleaseFocus: () => Promise<void>;
  /** 收起面板。 */
  onDismiss: () => void;
}

/** 最近记录展示条数（面板高度有限）。 */
const RECENT_LIMIT = 6;

/**
 * 快捷小窗的「记录」页：一行记下刚完成的事。
 *
 * 支持 `#项目 1.5h 缺陷 昨天` 这样的轻量语法；解析不出来的词原样留在标题里，
 * 保证任何时候按回车都能存进去。保存成功后面板保持展开，方便连续补记。
 */
export function QuickPanelWork({
  active,
  focused,
  onRequestFocus,
  onReleaseFocus,
  onDismiss,
}: QuickPanelWorkProps) {
  const { projects } = useWorkProjects();
  const today = useMemo(() => ({ from: startOfDay(), to: endOfDay() }), []);
  const { logs } = useWorkLogs(today.from, today.to);

  const [input, setInput] = useState("");
  const [feedback, setFeedback] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (active && focused) inputRef.current?.focus();
  }, [active, focused]);

  useEffect(() => {
    if (!active) setFeedback(null);
  }, [active]);

  const handleMouseDown = (event: MouseEvent<HTMLDivElement>) => {
    if (focused) return;
    event.preventDefault();
    void onRequestFocus().then((ok) => {
      if (ok) inputRef.current?.focus();
    });
  };

  const submit = async () => {
    const raw = input.trim();
    if (!raw) {
      setFeedback("写点什么再记录");
      return;
    }
    const draft = parseQuickLog(raw, projects);
    setSaving(true);
    try {
      await ipc.createWorkLog({
        title: draft.title,
        projectId: draft.projectId,
        kind: draft.kind,
        hours: draft.hours,
        finishedAt: draft.finishedAt,
      });
      const suffix = [
        draft.projectName ? `#${draft.projectName}` : "",
        draft.hours > 0 ? formatHours(draft.hours) : "",
        WORK_KIND_LABELS[draft.kind],
      ]
        .filter(Boolean)
        .join(" · ");
      setFeedback(`已记录：${draft.title}${suffix ? `（${suffix}）` : ""}`);
      setInput("");
      if (focused) void onReleaseFocus();
    } catch (cause) {
      setFeedback(`记录失败：${String(cause)}`);
    } finally {
      setSaving(false);
    }
  };

  const todayHours = logs.reduce((sum, log) => sum + Math.max(0, log.hours), 0);

  return (
    <div className="flex flex-col gap-2">
      <div className="flex flex-col gap-0.5">
        <h2 className="font-heading text-sm font-medium">工作记录</h2>
        <p className="text-xs text-muted-foreground">
          {focused
            ? "回车记录 · Esc 退出；支持 #项目 1.5h 缺陷 昨天"
            : "点输入框记下刚完成的事；支持 #项目、1.5h、缺陷、昨天"}
        </p>
      </div>

      <div className="flex items-center gap-1.5" onMouseDown={handleMouseDown}>
        <Input
          ref={inputRef}
          value={input}
          readOnly={!focused}
          spellCheck={false}
          placeholder="例如：修复登录超时 #prism3 1.5h 缺陷"
          className={cn("h-8 border-0 bg-muted/40 text-xs", !focused && "cursor-pointer")}
          onChange={(event) => setInput(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.nativeEvent.isComposing) {
              event.preventDefault();
              void submit();
            }
            if (event.key === "Escape") {
              event.preventDefault();
              onDismiss();
            }
          }}
        />
        <Button size="sm" disabled={saving || !input.trim()} onClick={() => void submit()}>
          记录
        </Button>
      </div>

      {projects.length > 0 && (
        <p className="truncate text-[11px] text-muted-foreground">
          项目：
          {projects.map((project) => `#${project.name}`).join("  ")}
        </p>
      )}

      <p className="min-h-4 text-[11px] text-muted-foreground">{feedback ?? " "}</p>

      <div className="flex items-center justify-between text-[11px] text-muted-foreground">
        <span>今日完成</span>
        <span className="tabular-nums">
          {logs.length} 项{logs.length > 0 ? ` · ${formatHours(todayHours)}` : ""}
        </span>
      </div>

      {logs.length === 0 ? (
        <p className="rounded-lg bg-muted/25 px-3 py-3 text-xs leading-relaxed text-muted-foreground">
          今天还没有记录。做完一件事就回来记一行，周报会自动汇总。
        </p>
      ) : (
        <ul className="flex max-h-56 flex-col gap-0.5 overflow-y-auto">
          {logs.slice(0, RECENT_LIMIT).map((log) => (
            <li
              key={log.id}
              className="flex items-start gap-2 rounded-md px-1.5 py-1 hover:bg-muted/40"
            >
              <CheckCircle2 className="mt-0.5 size-3 shrink-0 text-emerald-500" aria-hidden />
              <div className="min-w-0 flex-1">
                <p className="truncate text-xs font-medium">{log.title}</p>
                <p className="truncate text-[0.65rem] text-muted-foreground">
                  {[
                    formatClock(log.finishedAt),
                    log.projectName || "",
                    WORK_KIND_LABELS[log.kind],
                    log.hours > 0 ? formatHours(log.hours) : "",
                  ]
                    .filter(Boolean)
                    .join(" · ")}
                </p>
              </div>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
