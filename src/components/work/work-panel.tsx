import { useEffect, useMemo, useState } from "react";

import {
  CalendarClock,
  Check,
  CheckCircle2,
  FileText,
  Pencil,
  Plus,
  Trash2,
} from "lucide-react";
import { toast } from "sonner";

import { PageHeader } from "@/components/app/page-header";
import { WorkLogEditDialog } from "@/components/work/work-log-edit-dialog";
import { WorkReportDialog } from "@/components/work/work-report-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Textarea } from "@/components/ui/textarea";
import { useWorkLogs, useWorkPlans, useWorkProjects } from "@/hooks/use-work-logs";
import {
  ipc,
  type WorkLog,
  type WorkLogKind,
  type WorkLogStatus,
  type WorkProject,
} from "@/lib/ipc";
import { WORK_KIND_CLASS, WORK_KIND_LABELS, WORK_KIND_OPTIONS } from "@/lib/work-kinds";
import {
  dateInputValue,
  dayKey,
  endOfDay,
  endOfMonth,
  endOfWeek,
  formatClock,
  formatDayLabel,
  formatHours,
  startOfDay,
  startOfMonth,
  startOfWeek,
  timestampFromDateInput,
} from "@/lib/work-time";
import { cn } from "@/lib/utils";

type RangeKey = "today" | "week" | "month";

/** 左侧项目清单的筛选口径。 */
type WorkFilter =
  | { kind: "all" }
  | { kind: "project"; id: number }
  | { kind: "unassigned" };

const RANGE_LABELS: { value: RangeKey; label: string }[] = [
  { value: "today", label: "今天" },
  { value: "week", label: "本周" },
  { value: "month", label: "本月" },
];

/**
 * 主窗口「工作」页：左侧项目清单 + 右侧快速记录、统计与时间线。
 *
 * 记录是主角：新建只要求一行标题，项目 / 类型 / 工时都可以不填；
 * 周报按钮直接生成本周 Markdown，复制到剪贴板。
 *
 * `active` 供 hub 使用：切走 / 收起时关闭弹窗，避免下次呼出还挂着一个
 * 上次没关的编辑框或周报预览。主窗口始终传默认 `true`。
 */
export function WorkPanel({ active = true }: { active?: boolean }) {
  const { projects } = useWorkProjects();
  const [range, setRange] = useState<RangeKey>("today");

  const bounds = useMemo(() => {
    if (range === "today") return { from: startOfDay(), to: endOfDay() };
    if (range === "week") return { from: startOfWeek(), to: endOfWeek() };
    return { from: startOfMonth(), to: endOfMonth() };
  }, [range]);

  const { logs } = useWorkLogs(bounds.from, bounds.to);
  const { plans } = useWorkPlans();
  const week = useMemo(() => ({ from: startOfWeek(), to: endOfWeek() }), []);
  const { logs: weekLogs } = useWorkLogs(week.from, week.to);

  /** 当前记录表单是「记完成」还是「列计划」。 */
  const [entryMode, setEntryMode] = useState<WorkLogStatus>("done");
  const [title, setTitle] = useState("");
  const [detail, setDetail] = useState("");
  const [projectId, setProjectId] = useState<number | null>(null);
  const [kind, setKind] = useState<WorkLogKind>("dev");
  const [hours, setHours] = useState("");
  const [date, setDate] = useState(() => dateInputValue(Math.floor(Date.now() / 1000)));
  const [saving, setSaving] = useState(false);

  const [newProject, setNewProject] = useState("");
  const [confirmProjectId, setConfirmProjectId] = useState<number | null>(null);
  const [confirmLogId, setConfirmLogId] = useState<number | null>(null);
  const [editing, setEditing] = useState<WorkLog | null>(null);
  const [reportOpen, setReportOpen] = useState(false);
  /** 左侧清单筛选：全部 / 某个项目 / 未归属。 */
  const [filter, setFilter] = useState<WorkFilter>({ kind: "all" });
  /** 正在就地重命名的项目 id 与输入值。 */
  const [renamingId, setRenamingId] = useState<number | null>(null);
  const [renameValue, setRenameValue] = useState("");

  // hub 切走 / 收起时把弹窗与就地编辑全部收掉：窗口只是隐藏，组件不会卸载。
  useEffect(() => {
    if (active) return;
    setEditing(null);
    setReportOpen(false);
    setConfirmProjectId(null);
    setConfirmLogId(null);
    setRenamingId(null);
  }, [active]);

  /** 当前筛选后的记录：统计与时间线都基于它，项目清单的计数仍用全量。 */
  const filteredLogs = useMemo(() => {
    if (filter.kind === "project") {
      return logs.filter((log) => log.projectId === filter.id);
    }
    if (filter.kind === "unassigned") {
      return logs.filter((log) => log.projectId == null);
    }
    return logs;
  }, [logs, filter]);

  /** 未来计划按同一筛选口径展示（看某个项目时只列该项目的计划）。 */
  const filteredPlans = useMemo(() => {
    if (filter.kind === "project") {
      return plans.filter((plan) => plan.projectId === filter.id);
    }
    if (filter.kind === "unassigned") {
      return plans.filter((plan) => plan.projectId == null);
    }
    return plans;
  }, [plans, filter]);

  const totalHours = filteredLogs.reduce((sum, log) => sum + Math.max(0, log.hours), 0);
  const coveredProjects = new Set(filteredLogs.map((log) => log.projectName || "未归属")).size;
  const unassignedCount = logs.filter((log) => log.projectId == null).length;

  const grouped = useMemo(() => {
    const map = new Map<string, WorkLog[]>();
    for (const log of filteredLogs) {
      const key = dayKey(log.finishedAt);
      const bucket = map.get(key);
      if (bucket) bucket.push(log);
      else map.set(key, [log]);
    }
    return [...map.values()];
  }, [filteredLogs]);

  const countByProject = useMemo(() => {
    const map = new Map<number, number>();
    for (const log of logs) {
      if (log.projectId == null) continue;
      map.set(log.projectId, (map.get(log.projectId) ?? 0) + 1);
    }
    return map;
  }, [logs]);

  const submit = async () => {
    const trimmed = title.trim();
    if (!trimmed) {
      toast.error(entryMode === "planned" ? "请填写计划内容" : "请填写完成内容");
      return;
    }
    const parsedHours = hours.trim() === "" ? 0 : Number(hours);
    if (!Number.isFinite(parsedHours) || parsedHours < 0 || parsedHours > 24) {
      toast.error("工时应在 0 到 24 小时之间");
      return;
    }
    setSaving(true);
    try {
      await ipc.createWorkLog({
        title: trimmed,
        detail,
        projectId,
        kind,
        status: entryMode,
        hours: parsedHours,
        finishedAt: timestampFromDateInput(date, Math.floor(Date.now() / 1000)),
      });
      toast.success(entryMode === "planned" ? "已加入未来计划" : "已记录");
      setTitle("");
      setDetail("");
      setHours("");
      setDate(dateInputValue(Math.floor(Date.now() / 1000)));
    } catch (cause) {
      toast.error(`${entryMode === "planned" ? "添加计划" : "记录"}失败：${String(cause)}`);
    } finally {
      setSaving(false);
    }
  };

  /** 一键把计划转成完成记录（完成时间 = 现在）。 */
  const completePlan = async (plan: WorkLog) => {
    try {
      await ipc.completeWorkPlan(plan.id);
      setConfirmLogId(null);
      toast.success(`已完成「${plan.title}」`);
    } catch (cause) {
      toast.error(`标记完成失败：${String(cause)}`);
    }
  };

  const addProject = async () => {
    const name = newProject.trim();
    if (!name) return;
    try {
      await ipc.createWorkProject(name);
      setNewProject("");
      toast.success(`已添加项目「${name}」`);
    } catch (cause) {
      toast.error(`添加项目失败：${String(cause)}`);
    }
  };

  const removeProject = async (project: WorkProject) => {
    try {
      await ipc.deleteWorkProject(project.id);
      if (filter.kind === "project" && filter.id === project.id) setFilter({ kind: "all" });
      setConfirmProjectId(null);
      toast.success(`已删除项目「${project.name}」，原有记录保留`);
    } catch (cause) {
      toast.error(`删除项目失败：${String(cause)}`);
    }
  };

  const startRename = (project: WorkProject) => {
    setConfirmProjectId(null);
    setRenamingId(project.id);
    setRenameValue(project.name);
  };

  const cancelRename = () => {
    setRenamingId(null);
    setRenameValue("");
  };

  const commitRename = async () => {
    const id = renamingId;
    if (id === null) return;
    const name = renameValue.trim();
    const current = projects.find((project) => project.id === id);
    if (!name || name === current?.name) {
      cancelRename();
      return;
    }
    try {
      await ipc.renameWorkProject(id, name);
      toast.success(`已重命名为「${name}」`);
      cancelRename();
    } catch (cause) {
      toast.error(`重命名失败：${String(cause)}`);
    }
  };

  const removeLog = async (log: WorkLog) => {
    try {
      await ipc.deleteWorkLog(log.id);
      setConfirmLogId(null);
      toast.success("已删除这条记录");
    } catch (cause) {
      toast.error(`删除失败：${String(cause)}`);
    }
  };

  return (
    <div className="flex h-full min-h-0 gap-4">
      <aside className="glass-card flex w-56 shrink-0 flex-col rounded-xl p-3">
        <div className="flex items-center justify-between">
          <h2 className="text-xs font-medium text-muted-foreground">项目</h2>
          <span className="text-[11px] text-muted-foreground tabular-nums">{projects.length}</span>
        </div>

        <div className="mt-2 flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto">
          <FilterRow
            label="全部"
            count={logs.length}
            active={filter.kind === "all"}
            onClick={() => setFilter({ kind: "all" })}
          />

          {projects.map((project) => {
            const active = filter.kind === "project" && filter.id === project.id;
            return (
              <div
                key={project.id}
                className={cn(
                  "group flex items-center gap-1 rounded-md px-2 py-1.5 text-xs hover:bg-muted/40",
                  active && "bg-brand-soft",
                )}
              >
                {renamingId === project.id ? (
                  <Input
                    autoFocus
                    value={renameValue}
                    spellCheck={false}
                    className="h-6 min-w-0 flex-1 px-1.5 text-xs"
                    onChange={(event) => setRenameValue(event.target.value)}
                    onKeyDown={(event) => {
                      if (event.key === "Enter") {
                        event.preventDefault();
                        void commitRename();
                      } else if (event.key === "Escape") {
                        event.preventDefault();
                        cancelRename();
                      }
                    }}
                    onBlur={cancelRename}
                  />
                ) : (
                  <button
                    type="button"
                    title={`只看「${project.name}」的记录`}
                    className={cn(
                      "min-w-0 flex-1 truncate text-left",
                      active && "font-medium text-brand",
                    )}
                    onClick={() => setFilter({ kind: "project", id: project.id })}
                  >
                    {project.name}
                  </button>
                )}
                <span className="shrink-0 text-[11px] text-muted-foreground tabular-nums">
                  {countByProject.get(project.id) ?? 0}
                </span>
                {renamingId !== project.id &&
                  (confirmProjectId === project.id ? (
                    <Button
                      variant="destructive"
                      size="icon-xs"
                      title="确认删除（记录保留）"
                      onClick={() => void removeProject(project)}
                    >
                      <Check />
                    </Button>
                  ) : (
                    <>
                      <Button
                        variant="ghost"
                        size="icon-xs"
                        className="opacity-0 group-hover:opacity-100"
                        title="重命名项目"
                        aria-label={`重命名项目 ${project.name}`}
                        onClick={() => startRename(project)}
                      >
                        <Pencil />
                      </Button>
                      <Button
                        variant="ghost"
                        size="icon-xs"
                        className="opacity-0 group-hover:opacity-100"
                        title="删除项目（记录保留）"
                        aria-label={`删除项目 ${project.name}`}
                        onClick={() => setConfirmProjectId(project.id)}
                      >
                        <Trash2 />
                      </Button>
                    </>
                  ))}
              </div>
            );
          })}

          {unassignedCount > 0 && (
            <FilterRow
              label="未归属"
              count={unassignedCount}
              active={filter.kind === "unassigned"}
              onClick={() => setFilter({ kind: "unassigned" })}
            />
          )}

          {projects.length === 0 && (
            <p className="mt-1 rounded-lg bg-muted/30 px-2.5 py-2 text-[11px] leading-relaxed text-muted-foreground">
              还没有项目。工作记录可以先不归属项目，之后再补。
            </p>
          )}
        </div>

        <div className="mt-3 flex items-center gap-1 border-t border-border/50 pt-2">
          <Input
            value={newProject}
            spellCheck={false}
            placeholder="新建项目"
            className="h-7 text-xs"
            onChange={(event) => setNewProject(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") void addProject();
            }}
          />
          <Button
            variant="ghost"
            size="icon-xs"
            title="添加项目"
            disabled={!newProject.trim()}
            onClick={() => void addProject()}
          >
            <Plus />
          </Button>
        </div>
      </aside>

      <div className="flex min-w-0 flex-1 flex-col gap-3">
        <PageHeader
          title="工作记录"
          description="记下每天完成了什么；周报由记录自动汇总，不用再回忆。"
          actions={
            <Button size="sm" className="gap-1.5" onClick={() => setReportOpen(true)}>
              <FileText className="size-3.5" aria-hidden />
              生成本周报告
            </Button>
          }
        />

        <div className="glass-card rounded-xl p-3">
          <div className="mb-2 flex flex-wrap items-center gap-1.5">
            <Button
              variant={entryMode === "done" ? "secondary" : "ghost"}
              size="xs"
              onClick={() => setEntryMode("done")}
            >
              完成记录
            </Button>
            <Button
              variant={entryMode === "planned" ? "secondary" : "ghost"}
              size="xs"
              onClick={() => setEntryMode("planned")}
            >
              <CalendarClock className="size-3.5" aria-hidden />
              计划未来
            </Button>
            <span className="text-[11px] text-muted-foreground">
              {entryMode === "planned"
                ? "先列出来，完成后在计划列表点 ✓ 转为完成记录"
                : "记录已经做完的事；周报自动汇总"}
            </span>
          </div>

          <div className="flex items-center gap-2">
            <Input
              value={title}
              spellCheck={false}
              placeholder={
                entryMode === "planned"
                  ? "计划做什么？例如：下周补登录模块测试"
                  : "完成了什么？例如：修复登录超时"
              }
              className="h-8 flex-1 text-xs"
              onChange={(event) => setTitle(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter" && !event.nativeEvent.isComposing) void submit();
              }}
            />
            <Button size="sm" disabled={saving || !title.trim()} onClick={() => void submit()}>
              {entryMode === "planned" ? "加入计划" : "记录"}
            </Button>
          </div>

          <div className="mt-2 flex flex-wrap items-center gap-2">
            <select
              className="h-7 rounded-md border border-input bg-background px-1.5 text-xs"
              value={projectId ?? ""}
              onChange={(event) =>
                setProjectId(event.target.value === "" ? null : Number(event.target.value))
              }
            >
              <option value="">未归属项目</option>
              {projects.map((project) => (
                <option key={project.id} value={project.id}>
                  {project.name}
                </option>
              ))}
            </select>

            <select
              className="h-7 rounded-md border border-input bg-background px-1.5 text-xs"
              value={kind}
              onChange={(event) => setKind(event.target.value as WorkLogKind)}
            >
              {WORK_KIND_OPTIONS.map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </select>

            <Input
              type="number"
              min={0}
              max={24}
              step={0.5}
              placeholder="工时"
              className="h-7 w-20 text-xs"
              value={hours}
              onChange={(event) => setHours(event.target.value)}
            />

            <Input
              type="date"
              title={entryMode === "planned" ? "计划日期" : "完成日期"}
              className="h-7 w-32 text-xs"
              value={date}
              onChange={(event) => setDate(event.target.value)}
            />
          </div>

          <Textarea
            value={detail}
            placeholder={
              entryMode === "planned"
                ? "计划说明（可选）：目标、依赖、验收标准…"
                : "补充说明（可选）：结果、链接、影响范围…"
            }
            className="mt-2 min-h-14 text-xs"
            onChange={(event) => setDetail(event.target.value)}
          />
        </div>

        <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
          <StatBox label="完成项" value={String(filteredLogs.length)} />
          <StatBox label="合计工时" value={totalHours > 0 ? formatHours(totalHours) : "0h"} />
          <StatBox label="涉及项目" value={String(coveredProjects)} />
          <StatBox label="未来计划" value={String(filteredPlans.length)} />
        </div>

        <PlanList
          plans={filteredPlans}
          confirmId={confirmLogId}
          onComplete={(plan) => void completePlan(plan)}
          onEdit={(plan) => setEditing(plan)}
          onDelete={(plan) => setConfirmLogId(plan.id)}
          onConfirmDelete={(plan) => void removeLog(plan)}
        />

        <div className="glass-card flex min-h-0 flex-1 flex-col rounded-xl p-3">
          <div className="flex shrink-0 items-center justify-between gap-3">
            <Tabs value={range} onValueChange={(value) => setRange(value as RangeKey)}>
              <TabsList>
                {RANGE_LABELS.map((item) => (
                  <TabsTrigger key={item.value} value={item.value} className="px-3 text-xs">
                    {item.label}
                  </TabsTrigger>
                ))}
              </TabsList>
            </Tabs>
            <span className="text-[11px] text-muted-foreground tabular-nums">
              {filteredLogs.length} 项
              {filteredLogs.length > 0 && totalHours > 0 ? ` · ${formatHours(totalHours)}` : ""}
            </span>
          </div>

          <div className="mt-2 flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto pr-0.5">
            {filteredLogs.length === 0 ? (
              <div className="flex flex-1 flex-col items-center justify-center gap-1.5 text-center">
                <p className="text-sm font-medium">
                  {filter.kind === "project"
                    ? "这个项目在这段时间还没有记录"
                    : filter.kind === "unassigned"
                      ? "这段时间没有未归属的记录"
                      : "这段时间还没有记录"}
                </p>
                <p className="max-w-sm text-xs text-muted-foreground">
                  在上面的输入框写一行就能记下；也可以在快捷小窗的「记录」页随手记。
                </p>
              </div>
            ) : (
              grouped.map((entries) => {
                const dayHours = entries.reduce((sum, log) => sum + Math.max(0, log.hours), 0);
                return (
                  <section key={dayKey(entries[0].finishedAt)} className="flex flex-col gap-0.5">
                    <h3 className="flex items-baseline gap-2 px-2 text-[11px] font-medium text-muted-foreground">
                      {formatDayLabel(entries[0].finishedAt)}
                      <span className="font-normal tabular-nums">
                        {entries.length} 项
                        {dayHours > 0 ? ` · ${formatHours(dayHours)}` : ""}
                      </span>
                    </h3>
                  {entries.map((log) => (
                    <div
                      key={log.id}
                      className="group flex items-start gap-3 rounded-lg px-2 py-1.5 hover:bg-muted/30"
                    >
                      <span className="w-10 shrink-0 pt-0.5 text-[11px] text-muted-foreground tabular-nums">
                        {formatClock(log.finishedAt)}
                      </span>
                      <div className="min-w-0 flex-1">
                        <div className="flex flex-wrap items-center gap-1.5">
                          <span className="text-sm font-medium break-words">{log.title}</span>
                          {log.projectName && (
                            <Badge variant="outline" className="h-4 px-1.5 text-[10px]">
                              {log.projectName}
                            </Badge>
                          )}
                          <span
                            className={cn(
                              "inline-flex h-4 items-center rounded-full px-1.5 text-[10px] font-medium",
                              WORK_KIND_CLASS[log.kind],
                            )}
                          >
                            {WORK_KIND_LABELS[log.kind]}
                          </span>
                          <span className="ml-auto shrink-0 text-[11px] text-muted-foreground tabular-nums">
                            {formatHours(log.hours)}
                          </span>
                        </div>
                        {log.detail.trim() && (
                          <p className="mt-0.5 line-clamp-2 text-xs text-muted-foreground">
                            {log.detail}
                          </p>
                        )}
                      </div>

                      <div className="flex shrink-0 items-center gap-0.5 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100">
                        {confirmLogId === log.id ? (
                          <Button
                            variant="destructive"
                            size="icon-xs"
                            title="确认删除"
                            onClick={() => void removeLog(log)}
                          >
                            <Check />
                          </Button>
                        ) : (
                          <>
                            <Button
                              variant="ghost"
                              size="icon-xs"
                              title="编辑"
                              onClick={() => setEditing(log)}
                            >
                              <Pencil />
                            </Button>
                            <Button
                              variant="ghost"
                              size="icon-xs"
                              title="删除"
                              onClick={() => setConfirmLogId(log.id)}
                            >
                              <Trash2 />
                            </Button>
                          </>
                        )}
                      </div>
                    </div>
                  ))}
                  </section>
                );
              })
            )}
          </div>
        </div>
      </div>

      <WorkLogEditDialog
        log={editing}
        projects={projects}
        onOpenChange={(open) => {
          if (!open) setEditing(null);
        }}
        onSaved={() => setEditing(null)}
      />

      <WorkReportDialog
        open={reportOpen}
        onOpenChange={setReportOpen}
        logs={weekLogs}
        plans={plans}
        from={week.from}
        to={week.to}
      />
    </div>
  );
}

function PlanList({
  plans,
  confirmId,
  onComplete,
  onEdit,
  onDelete,
  onConfirmDelete,
}: {
  plans: WorkLog[];
  confirmId: number | null;
  onComplete: (plan: WorkLog) => void;
  onEdit: (plan: WorkLog) => void;
  onDelete: (plan: WorkLog) => void;
  onConfirmDelete: (plan: WorkLog) => void;
}) {
  return (
    <div className="glass-card flex max-h-52 shrink-0 flex-col rounded-xl p-3">
      <div className="flex shrink-0 items-center justify-between">
        <span className="flex items-center gap-1.5 text-xs font-medium">
          <CalendarClock className="size-3.5 text-brand" aria-hidden />
          未来计划
        </span>
        <span className="text-[11px] text-muted-foreground tabular-nums">{plans.length} 项</span>
      </div>

      <div className="mt-2 flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto">
        {plans.length === 0 ? (
          <p className="rounded-lg bg-muted/30 px-2.5 py-2 text-[11px] leading-relaxed text-muted-foreground">
            还没有计划。把要做的事先写进来，完成后点 ✓ 转为完成记录。
          </p>
        ) : (
          plans.map((plan) => {
            const overdue = plan.finishedAt < startOfDay();
            return (
              <div
                key={plan.id}
                className="group flex items-start gap-2 rounded-md px-2 py-1.5 hover:bg-muted/30"
              >
                <span
                  className={cn(
                    "w-14 shrink-0 pt-0.5 text-[11px] tabular-nums",
                    overdue ? "text-destructive" : "text-muted-foreground",
                  )}
                  title={overdue ? "计划日期已过" : undefined}
                >
                  {formatDayLabel(plan.finishedAt)}
                </span>
                <div className="min-w-0 flex-1">
                  <div className="flex flex-wrap items-center gap-1.5">
                    <span className="text-sm font-medium break-words">{plan.title}</span>
                    {plan.projectName && (
                      <Badge variant="outline" className="h-4 px-1.5 text-[10px]">
                        {plan.projectName}
                      </Badge>
                    )}
                    <span
                      className={cn(
                        "inline-flex h-4 items-center rounded-full px-1.5 text-[10px] font-medium",
                        WORK_KIND_CLASS[plan.kind],
                      )}
                    >
                      {WORK_KIND_LABELS[plan.kind]}
                    </span>
                    <span className="ml-auto shrink-0 text-[11px] text-muted-foreground tabular-nums">
                      {formatHours(plan.hours)}
                    </span>
                  </div>
                  {plan.detail.trim() && (
                    <p className="mt-0.5 line-clamp-2 text-xs text-muted-foreground">
                      {plan.detail}
                    </p>
                  )}
                </div>

                <div className="flex shrink-0 items-center gap-0.5 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100">
                  {confirmId === plan.id ? (
                    <Button
                      variant="destructive"
                      size="icon-xs"
                      title="确认删除"
                      onClick={() => onConfirmDelete(plan)}
                    >
                      <Check />
                    </Button>
                  ) : (
                    <>
                      <Button
                        variant="ghost"
                        size="icon-xs"
                        title="标记完成（转为完成记录）"
                        onClick={() => onComplete(plan)}
                      >
                        <CheckCircle2 />
                      </Button>
                      <Button
                        variant="ghost"
                        size="icon-xs"
                        title="编辑"
                        onClick={() => onEdit(plan)}
                      >
                        <Pencil />
                      </Button>
                      <Button
                        variant="ghost"
                        size="icon-xs"
                        title="删除"
                        onClick={() => onDelete(plan)}
                      >
                        <Trash2 />
                      </Button>
                    </>
                  )}
                </div>
              </div>
            );
          })
        )}
      </div>
    </div>
  );
}

function FilterRow({
  label,
  count,
  active,
  onClick,
}: {
  label: string;
  count: number;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "flex items-center gap-1 rounded-md px-2 py-1.5 text-xs hover:bg-muted/40",
        active && "bg-brand-soft font-medium text-brand",
      )}
    >
      <span className="min-w-0 flex-1 truncate text-left">{label}</span>
      <span className="shrink-0 text-[11px] text-muted-foreground tabular-nums">{count}</span>
    </button>
  );
}

function StatBox({ label, value }: { label: string; value: string }) {
  return (
    <div className="glass-card flex flex-col rounded-xl px-3 py-2">
      <span className="font-heading text-lg font-semibold tabular-nums">{value}</span>
      <span className="text-[11px] text-muted-foreground">{label}</span>
    </div>
  );
}
