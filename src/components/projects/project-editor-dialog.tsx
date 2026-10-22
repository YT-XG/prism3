import { useState } from "react";

import { open as openDirectoryDialog } from "@tauri-apps/plugin-dialog";
import { FolderOpen, Plus, Trash2 } from "lucide-react";

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
import type { ProjectCommand, ProjectEntry } from "@/lib/ipc";
import { cn } from "@/lib/utils";

/** 对话框产出的草稿：id 与持久化由调用方决定（新建 / 编辑共用）。 */
export type ProjectDraft = Omit<ProjectEntry, "id">;

interface ProjectEditorDialogProps {
  /** 要编辑的项目；null = 新建。 */
  project: ProjectEntry | null;
  onClose: () => void;
  /** 返回的 Promise 失败 = 保存失败（调用方已提示），对话框保持打开。 */
  onSave: (draft: ProjectDraft) => Promise<void>;
}

/** 目录字符串的最后一段，用作新建项目时的默认名称。 */
function folderNameOf(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? "";
}

/** 项目新增 / 编辑对话框：名称 + 目录 + 若干启动命令预设。 */
export function ProjectEditorDialog({ project, onClose, onSave }: ProjectEditorDialogProps) {
  const [name, setName] = useState(project?.name ?? "");
  const [path, setPath] = useState(project?.path ?? "");
  const [commands, setCommands] = useState<ProjectCommand[]>(project?.commands ?? []);
  const [saving, setSaving] = useState(false);

  const nameOk = name.trim() !== "";
  const pathOk = path.trim() !== "";
  // 整行留空视为「不要这条」，保存时丢弃；只填了一半则拦下来。
  const partialCommand = commands.some(
    (command) =>
      (command.name.trim() === "") !== (command.command.trim() === ""),
  );
  const canSave = nameOk && pathOk && !partialCommand && !saving;

  const pickDirectory = async () => {
    const picked = await openDirectoryDialog({
      directory: true,
      multiple: false,
      title: "选择项目目录",
    });
    if (typeof picked !== "string") return;
    setPath(picked);
    if (!name.trim()) setName(folderNameOf(picked));
  };

  const updateCommand = (index: number, patch: Partial<ProjectCommand>) => {
    setCommands((current) =>
      current.map((command, i) => (i === index ? { ...command, ...patch } : command)),
    );
  };

  const handleSave = async () => {
    if (!canSave) return;
    setSaving(true);
    try {
      await onSave({ name, path, commands });
      onClose();
    } catch {
      // 调用方已经把失败原因 toast 出来；留在对话框里让用户改。
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog open onOpenChange={(next) => (!next ? onClose() : undefined)}>
      <DialogContent className="sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>{project ? "编辑项目" : "添加项目"}</DialogTitle>
          <DialogDescription>
            目录与启动命令都保存在本机配置里；启动时会新开一个 cmd 窗口。
          </DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-4">
          <div className="grid gap-1.5">
            <span className="text-xs font-medium text-muted-foreground">名称</span>
            <Input
              value={name}
              autoFocus
              spellCheck={false}
              placeholder="例如 Prism3"
              onChange={(event) => setName(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") void handleSave();
              }}
            />
          </div>

          <div className="grid gap-1.5">
            <span className="text-xs font-medium text-muted-foreground">项目目录</span>
            <div className="flex items-center gap-2">
              <Input
                value={path}
                spellCheck={false}
                placeholder="D:\work\my-project"
                onChange={(event) => setPath(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter") void handleSave();
                }}
              />
              <Button type="button" variant="outline" size="sm" onClick={() => void pickDirectory()}>
                <FolderOpen data-icon="inline-start" />
                选择
              </Button>
            </div>
          </div>

          <div className="grid gap-2">
            <div className="flex items-center justify-between gap-2">
              <span className="text-xs font-medium text-muted-foreground">
                启动命令（可选，可加多条）
              </span>
              <Button
                type="button"
                variant="ghost"
                size="xs"
                onClick={() => setCommands((current) => [...current, { name: "", command: "" }])}
              >
                <Plus data-icon="inline-start" />
                添加命令
              </Button>
            </div>

            {commands.length === 0 ? (
              <p className="rounded-lg border border-dashed border-border/70 px-3 py-2 text-xs text-muted-foreground">
                没有启动命令时只能「打开文件夹」；填一条（如 dev → pnpm dev）就能一键起服务。
              </p>
            ) : (
              <div className="flex flex-col gap-2">
                {commands.map((command, index) => (
                  <div
                    key={index}
                    className={cn(
                      "flex items-center gap-2 rounded-lg border px-2 py-1.5",
                      (command.name.trim() === "") !== (command.command.trim() === "")
                        ? "border-destructive/50 bg-destructive/5"
                        : "border-border/60 bg-muted/30",
                    )}
                  >
                    <Input
                      value={command.name}
                      spellCheck={false}
                      placeholder="dev"
                      aria-label="命令名称"
                      className="h-7 w-24 shrink-0 text-xs"
                      onChange={(event) => updateCommand(index, { name: event.target.value })}
                    />
                    <Input
                      value={command.command}
                      spellCheck={false}
                      placeholder="pnpm dev"
                      aria-label="命令行"
                      className="h-7 flex-1 font-mono text-xs"
                      onChange={(event) => updateCommand(index, { command: event.target.value })}
                      onKeyDown={(event) => {
                        if (event.key === "Enter") void handleSave();
                      }}
                    />
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon-xs"
                      aria-label="删除这条命令"
                      onClick={() =>
                        setCommands((current) => current.filter((_, i) => i !== index))
                      }
                    >
                      <Trash2 />
                    </Button>
                  </div>
                ))}
              </div>
            )}

            {partialCommand && (
              <p className="text-xs text-destructive">
                每条命令的「名称」和「命令」都要填；整行留空会自动忽略。
              </p>
            )}
          </div>
        </div>

        <DialogFooter>
          <Button type="button" variant="outline" size="sm" onClick={onClose}>
            取消
          </Button>
          <Button type="button" size="sm" disabled={!canSave} onClick={() => void handleSave()}>
            {saving ? "保存中…" : "保存"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
