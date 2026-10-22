import { useState } from "react";

import { FolderGit2, FolderOpen, FolderPlus, Pencil, Play, Plus, Trash2 } from "lucide-react";

import {
  ProjectEditorDialog,
  type ProjectDraft,
} from "@/components/projects/project-editor-dialog";
import { Button } from "@/components/ui/button";
import { Card, CardAction, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { useProjects } from "@/hooks/use-projects";
import type { ProjectEntry } from "@/lib/ipc";

/**
 * 主窗口「项目」页：本地项目的列表、增删改与一键启动。
 *
 * 页面只在被选中时挂载（`AppShell` 按视图分支渲染），列表直接读配置、不额外拉数据。
 */
export function ProjectsPanel() {
  const { projects, addProject, updateProject, removeProject, openFolder, runCommand } =
    useProjects();

  /** `undefined` = 关闭；`null` = 新建；否则是编辑中的项目。 */
  const [editor, setEditor] = useState<ProjectEntry | null | undefined>(undefined);
  /** 正在二次确认删除的项目 id。 */
  const [pendingDelete, setPendingDelete] = useState<string | null>(null);

  const handleSave = async (draft: ProjectDraft) => {
    if (editor) {
      await updateProject({ ...draft, id: editor.id });
    } else {
      await addProject(draft);
    }
  };

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <h1 className="font-heading text-lg font-medium">项目</h1>
          <p className="text-sm text-muted-foreground">
            把常开的本地服务收进来：一键打开目录，或在新 cmd 窗口里跑启动命令。
          </p>
        </div>
        <Button size="sm" onClick={() => setEditor(null)}>
          <Plus data-icon="inline-start" />
          添加项目
        </Button>
      </div>

      {projects.length === 0 ? (
        <div className="flex flex-col items-center justify-center gap-3 rounded-xl border border-dashed border-border/70 px-6 py-14 text-center">
          <span className="flex size-10 items-center justify-center rounded-xl bg-brand-soft text-brand">
            <FolderPlus className="size-5" aria-hidden="true" />
          </span>
          <div>
            <p className="text-sm font-medium">还没有项目</p>
            <p className="mt-1 text-xs text-muted-foreground">
              把每天都要启动的服务加进来，以后不用再翻目录、开 cmd。
            </p>
          </div>
          <Button size="sm" onClick={() => setEditor(null)}>
            <Plus data-icon="inline-start" />
            添加第一个项目
          </Button>
        </div>
      ) : (
        <div className="grid grid-cols-1 gap-3 lg:grid-cols-2 2xl:grid-cols-3">
          {projects.map((project) => (
            <Card key={project.id} size="sm" className="gap-2.5">
              <CardHeader className="grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-2.5">
                <span className="col-start-1 row-start-1 flex size-8 items-center justify-center rounded-lg bg-brand-soft text-brand">
                  <FolderGit2 className="size-4" aria-hidden="true" />
                </span>
                <div className="col-start-2 row-start-1 min-w-0">
                  <CardTitle className="truncate">{project.name}</CardTitle>
                  <p
                    className="truncate text-xs text-muted-foreground"
                    title={project.path}
                  >
                    {project.path}
                  </p>
                </div>
                <CardAction className="col-start-3 row-span-1 row-start-1 self-center">
                  <span className="text-[0.65rem] text-muted-foreground tabular-nums">
                    {project.commands.length > 0 ? `${project.commands.length} 个命令` : "仅目录"}
                  </span>
                </CardAction>
              </CardHeader>

              <CardContent className="flex flex-wrap items-center gap-1.5">
                <Button
                  variant="outline"
                  size="xs"
                  title={`在资源管理器中打开 ${project.path}`}
                  onClick={() => void openFolder(project)}
                >
                  <FolderOpen data-icon="inline-start" />
                  打开目录
                </Button>

                {project.commands.map((command, index) => (
                  <Button
                    key={`${command.name}-${index}`}
                    variant="secondary"
                    size="xs"
                    title={command.command}
                    onClick={() => void runCommand(project, command)}
                  >
                    <Play data-icon="inline-start" />
                    <span className="max-w-36 truncate">{command.name || command.command}</span>
                  </Button>
                ))}

                {project.commands.length === 0 && (
                  <span className="text-[0.7rem] text-muted-foreground">
                    未配置启动命令
                  </span>
                )}

                <div className="ml-auto flex items-center gap-0.5">
                  <Button
                    variant="ghost"
                    size="icon-xs"
                    aria-label={`编辑 ${project.name}`}
                    title="编辑"
                    onClick={() => setEditor(project)}
                  >
                    <Pencil />
                  </Button>

                  {pendingDelete === project.id ? (
                    <>
                      <Button
                        variant="destructive"
                        size="xs"
                        onClick={() => {
                          void removeProject(project.id);
                          setPendingDelete(null);
                          if (editor?.id === project.id) setEditor(undefined);
                        }}
                      >
                        确认删除
                      </Button>
                      <Button
                        variant="ghost"
                        size="xs"
                        onClick={() => setPendingDelete(null)}
                      >
                        取消
                      </Button>
                    </>
                  ) : (
                    <Button
                      variant="ghost"
                      size="icon-xs"
                      aria-label={`删除 ${project.name}`}
                      title="删除"
                      onClick={() => setPendingDelete(project.id)}
                    >
                      <Trash2 />
                    </Button>
                  )}
                </div>
              </CardContent>
            </Card>
          ))}
        </div>
      )}

      {editor !== undefined && (
        <ProjectEditorDialog
          project={editor}
          onClose={() => setEditor(undefined)}
          onSave={handleSave}
        />
      )}
    </div>
  );
}
