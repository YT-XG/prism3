import { useEffect, useState } from "react";

import { Trash2 } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { ipc, type WorkLog, type WorkLogKind, type WorkLogStatus, type WorkProject } from "@/lib/ipc";
import { WORK_KIND_OPTIONS } from "@/lib/work-kinds";
import { dateInputValue, timestampFromDateInput } from "@/lib/work-time";

interface WorkLogEditDialogProps {
  /** 要编辑的记录；`null` 表示关闭。 */
  log: WorkLog | null;
  projects: WorkProject[];
  onOpenChange: (open: boolean) => void;
  onSaved: () => void;
}

/** 编辑一条已完成记录：标题 / 项目 / 类型 / 工时 / 日期 / 说明，可删除。 */
export function WorkLogEditDialog({
  log,
  projects,
  onOpenChange,
  onSaved,
}: WorkLogEditDialogProps) {
  const [title, setTitle] = useState("");
  const [detail, setDetail] = useState("");
  const [projectId, setProjectId] = useState<number | null>(null);
  const [kind, setKind] = useState<WorkLogKind>("dev");
  const [status, setStatus] = useState<WorkLogStatus>("done");
  const [hours, setHours] = useState("");
  const [date, setDate] = useState("");
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const [saving, setSaving] = useState(false);

  // 每次换一条记录都重置表单（对话框是受控开合，组件本身不卸载）。
  useEffect(() => {
    if (!log) return;
    setTitle(log.title);
    setDetail(log.detail);
    setProjectId(log.projectId);
    setKind(log.kind);
    setStatus(log.status);
    setHours(log.hours > 0 ? String(log.hours) : "");
    setDate(dateInputValue(log.finishedAt));
    setConfirmingDelete(false);
    setSaving(false);
  }, [log]);

  const save = async () => {
    if (!log) return;
    const trimmed = title.trim();
    if (!trimmed) {
      toast.error("请填写标题");
      return;
    }
    const parsedHours = hours.trim() === "" ? 0 : Number(hours);
    if (!Number.isFinite(parsedHours) || parsedHours < 0 || parsedHours > 24) {
      toast.error("工时应在 0 到 24 小时之间");
      return;
    }
    setSaving(true);
    try {
      await ipc.updateWorkLog({
        id: log.id,
        title: trimmed,
        detail,
        projectId,
        kind,
        status,
        hours: parsedHours,
        finishedAt: timestampFromDateInput(date, log.finishedAt),
      });
      toast.success("已保存");
      onSaved();
      onOpenChange(false);
    } catch (cause) {
      toast.error(`保存失败：${String(cause)}`);
    } finally {
      setSaving(false);
    }
  };

  const remove = async () => {
    if (!log) return;
    try {
      await ipc.deleteWorkLog(log.id);
      toast.success("已删除这条记录");
      onSaved();
      onOpenChange(false);
    } catch (cause) {
      toast.error(`删除失败：${String(cause)}`);
    }
  };

  return (
    <Dialog open={log !== null} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{status === "planned" ? "编辑未来计划" : "编辑工作记录"}</DialogTitle>
          <DialogDescription>
            改完点保存；删除只影响这一条。把「计划中」改成「已完成」会把它转成完成记录。
          </DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-3">
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="work-edit-title">
              {status === "planned" ? "计划做什么" : "完成了什么"}
            </Label>
            <Input
              id="work-edit-title"
              value={title}
              spellCheck={false}
              onChange={(event) => setTitle(event.target.value)}
            />
          </div>

          <div className="flex flex-col gap-1.5">
            <Label htmlFor="work-edit-status">状态</Label>
            <select
              id="work-edit-status"
              className="h-8 rounded-md border border-input bg-background px-2 text-xs"
              value={status}
              onChange={(event) => setStatus(event.target.value as WorkLogStatus)}
            >
              <option value="done">已完成</option>
              <option value="planned">计划中（未来工作）</option>
            </select>
          </div>

          <div className="grid grid-cols-2 gap-3">
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="work-edit-project">项目</Label>
              <select
                id="work-edit-project"
                className="h-8 rounded-md border border-input bg-background px-2 text-xs"
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
            </div>
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="work-edit-kind">类型</Label>
              <select
                id="work-edit-kind"
                className="h-8 rounded-md border border-input bg-background px-2 text-xs"
                value={kind}
                onChange={(event) => setKind(event.target.value as WorkLogKind)}
              >
                {WORK_KIND_OPTIONS.map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </select>
            </div>
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="work-edit-hours">工时（小时）</Label>
              <Input
                id="work-edit-hours"
                type="number"
                min={0}
                max={24}
                step={0.5}
                placeholder="可留空"
                value={hours}
                onChange={(event) => setHours(event.target.value)}
              />
            </div>
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="work-edit-date">
                {status === "planned" ? "计划日期" : "完成日期"}
              </Label>
              <Input
                id="work-edit-date"
                type="date"
                value={date}
                onChange={(event) => setDate(event.target.value)}
              />
            </div>
          </div>

          <div className="flex flex-col gap-1.5">
            <Label htmlFor="work-edit-detail">补充说明</Label>
            <Textarea
              id="work-edit-detail"
              value={detail}
              placeholder="可选：结果、链接、影响范围…"
              className="min-h-16 text-xs"
              onChange={(event) => setDetail(event.target.value)}
            />
          </div>
        </div>

        <DialogFooter className="sm:justify-between">
          {confirmingDelete ? (
            <Button variant="destructive" size="sm" onClick={() => void remove()}>
              确认删除
            </Button>
          ) : (
            <Button variant="ghost" size="sm" onClick={() => setConfirmingDelete(true)}>
              <Trash2 className="size-3.5" aria-hidden />
              删除
            </Button>
          )}
          <div className="flex items-center gap-2">
            <Button variant="outline" size="sm" onClick={() => onOpenChange(false)}>
              取消
            </Button>
            <Button size="sm" disabled={saving} onClick={() => void save()}>
              保存
            </Button>
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
