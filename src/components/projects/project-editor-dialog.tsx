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
import { Textarea } from "@/components/ui/textarea";
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

/** 项目新增 / 编辑对话框：名称 + 可选项目目录 + 若干启动命令预设（命令可各自选目录）。 */
export function ProjectEditorDialog({ project, onClose, onSave }: ProjectEditorDialogProps) {
  const [name, setName] = useState(project?.name ?? "");
  // 项目根目录是**可选**的：留空时靠每条命令自己的工作目录启动；
  // 填写后既用于「打开目录」，也是命令没单独选目录时的默认工作目录。
  const [path, setPath] = useState(project?.path ?? "");
  // 老配置里没有 `dir`：补空串，避免受控输入拿到 undefined。
  const [commands, setCommands] = useState<ProjectCommand[]>(
    () =>
      project?.commands.map((command) => ({ ...command, dir: command.dir ?? "" })) ?? [],
  );
  const [saving, setSaving] = useState(false);

  const nameOk = name.trim() !== "";
  // 整行留空视为「不要这条」，保存时丢弃；只填了一半则拦下来。
  const partialCommand = commands.some(
    (command) =>
      (command.name.trim() === "") !== (command.command.trim() === ""),
  );
  // 没有项目根目录时，每条命令都必须自己选目录，否则启动时无处可跑。
  const missingCommandDir =
    path.trim() === "" &&
    commands.some(
      (command) =>
        (command.name.trim() !== "" || command.command.trim() !== "") &&
        command.dir.trim() === "",
    );
  const canSave = nameOk && !partialCommand && !missingCommandDir && !saving;

  const updateCommand = (index: number, patch: Partial<ProjectCommand>) => {
    setCommands((current) =>
      current.map((command, i) => (i === index ? { ...command, ...patch } : command)),
    );
  };

  /** 选择项目根目录（可选）；新建且名称还空着时顺手用目录末段补名。 */
  const pickProjectDirectory = async () => {
    const picked = await openDirectoryDialog({
      directory: true,
      multiple: false,
      title: "选择项目目录（可选）",
    });
    if (typeof picked !== "string") return;
    setPath(picked);
    if (!name.trim()) setName(folderNameOf(picked));
  };

  const pickCommandDirectory = async (index: number) => {
    const picked = await openDirectoryDialog({
      directory: true,
      multiple: false,
      title: "选择命令的工作目录",
    });
    if (typeof picked !== "string") return;
    updateCommand(index, {
      dir: picked,
      // 命令名留空时用目录末段补一个，省得再手打「前端 / 后端」。
      ...(commands[index]?.name.trim() === "" ? { name: folderNameOf(picked) } : {}),
    });
    if (!name.trim()) setName(folderNameOf(picked));
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
            命令在应用内运行，不弹命令行窗口；项目目录可选，每条命令也可以单独指定工作目录，
            前端 / 后端可以放在同一个项目里。
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
            <div className="flex items-center justify-between gap-2">
              <span className="text-xs font-medium text-muted-foreground">
                项目目录（可选）
              </span>
              {path.trim() !== "" && (
                <Button
                  type="button"
                  variant="ghost"
                  size="xs"
                  onClick={() => setPath("")}
                >
                  清除
                </Button>
              )}
            </div>
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="h-9 w-full justify-start overflow-hidden font-mono text-xs font-normal"
              aria-label="选择项目目录"
              title={path || "选择项目目录（可选）"}
              onClick={() => void pickProjectDirectory()}
            >
              <FolderOpen data-icon="inline-start" />
              <span className="truncate">{path || "选择文件夹（可选）"}</span>
            </Button>
            <p className="text-[0.7rem] text-muted-foreground">
              留空也可以：命令各自选目录即可。填写后会用于「打开目录」，也是命令没单独选目录时的默认工作目录。
            </p>
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
                onClick={() =>
                  setCommands((current) => [
                    ...current,
                    { name: "", command: "", dir: "" },
                  ])
                }
              >
                <Plus data-icon="inline-start" />
                添加命令
              </Button>
            </div>

            {commands.length === 0 ? (
              <p className="rounded-lg border border-dashed border-border/70 px-3 py-2 text-xs text-muted-foreground">
                没有启动命令时这个项目没有可执行的动作；填一条（如 前端 → pnpm dev）就能一键起服务。
              </p>
            ) : (
              <div className="flex flex-col gap-2">
                {commands.map((command, index) => (
                  <div
                    key={index}
                    className={cn(
                      "flex flex-col gap-1.5 rounded-lg border px-2 py-1.5",
                      (command.name.trim() === "") !== (command.command.trim() === "")
                        ? "border-destructive/50 bg-destructive/5"
                        : "border-border/60 bg-muted/30",
                    )}
                  >
                    <div className="flex items-center gap-2">
                      <Input
                        value={command.name}
                        spellCheck={false}
                        placeholder="前端"
                        aria-label="命令名称"
                        className="h-7 w-24 shrink-0 text-xs"
                        onChange={(event) =>
                          updateCommand(index, { name: event.target.value })
                        }
                      />
                      <Button
                        type="button"
                        variant="outline"
                        size="xs"
                        className="h-7 min-w-0 flex-1 justify-start overflow-hidden font-mono text-xs font-normal"
                        aria-label="选择命令工作目录"
                        title={command.dir || "选择这条命令的工作目录"}
                        onClick={() => void pickCommandDirectory(index)}
                      >
                        <FolderOpen data-icon="inline-start" />
                        <span className="truncate">{command.dir || "选择目录"}</span>
                      </Button>
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
                    <Textarea
                      value={command.command}
                      spellCheck={false}
                      rows={2}
                      placeholder={"pnpm dev\n也可以整段粘贴 README 里的启动命令，按行依次执行"}
                      aria-label="命令行"
                      className="min-h-0 resize-y font-mono text-xs leading-relaxed"
                      onChange={(event) =>
                        updateCommand(index, { command: event.target.value })
                      }
                    />
                  </div>
                ))}
              </div>
            )}

            <p className="text-[0.7rem] text-muted-foreground">
              命令支持多行：一行一条，在同一个会话里按顺序执行；空行和 # 开头的注释会被忽略。
            </p>

            {partialCommand && (
              <p className="text-xs text-destructive">
                每条命令的「名称」和「命令」都要填；整行留空会自动忽略。
              </p>
            )}

            {missingCommandDir && (
              <p className="text-xs text-destructive">
                项目没有根目录，每条命令都要先选择自己的工作目录。
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
