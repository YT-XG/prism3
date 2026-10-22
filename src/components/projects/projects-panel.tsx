import { useEffect, useState } from "react";

import {
  FolderGit2,
  FolderOpen,
  FolderPlus,
  Pencil,
  Play,
  Plus,
  Square,
  Terminal,
  Trash2,
} from "lucide-react";
import { toast } from "sonner";

import {
  ProjectEditorDialog,
  type ProjectDraft,
} from "@/components/projects/project-editor-dialog";
import { ProjectConsole } from "@/components/projects/project-console";
import { Button } from "@/components/ui/button";
import { projectCommandKey, useProjectRuns } from "@/hooks/use-project-runs";
import { useProjects } from "@/hooks/use-projects";
import type { ProjectEntry } from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { useAppStore } from "@/stores/app-store";

/**
 * 主窗口「项目」页：左列项目列表，右列选中项目的命令与实时输出。
 *
 * 命令由应用托管运行（不弹 cmd 窗口）：右列顶部是启停按钮，
 * 下方控制台占据剩余高度显示输出。页面走 `h-full` 分支（不进 ScrollArea），
 * 两列各自滚动。
 */
export function ProjectsPanel() {
  const { projects, addProject, updateProject, removeProject, openFolder } = useProjects();
  const {
    runs,
    selectedRun,
    selectedLines,
    openRun,
    closeRun,
    start,
    stop,
    clearLog,
    clearRun,
    findRunning,
    findLatest,
  } = useProjectRuns();

  /** `undefined` = 关闭；`null` = 新建；否则是编辑中的项目。 */
  const [editor, setEditor] = useState<ProjectEntry | null | undefined>(undefined);
  /** 正在二次确认删除的项目 id。 */
  const [pendingDelete, setPendingDelete] = useState<string | null>(null);
  /** 左列选中的项目；被删除后自动回落到第一个。 */
  const [activeId, setActiveId] = useState<string | null>(null);

  const activeProject = projects.find((project) => project.id === activeId) ?? projects[0] ?? null;
  /** 控制台只显示当前项目自己的运行记录，避免左列切换后串台。 */
  const consoleRun =
    selectedRun && activeProject && selectedRun.projectId === activeProject.id
      ? selectedRun
      : null;

  // 首页项目卡点「查看输出」：切过来时选中对应项目并展开那条记录的控制台。
  useEffect(() => {
    const pending = useAppStore.getState().consumePendingProjectRun();
    if (!pending) return;
    setActiveId(pending.projectId);
    if (pending.runId) void openRun(pending.runId);
    else closeRun();
  }, [closeRun, openRun]);

  const handleSave = async (draft: ProjectDraft) => {
    if (editor) {
      await updateProject({ ...draft, id: editor.id });
    } else {
      await addProject(draft);
    }
  };

  /** 删除项目前先把它的命令停掉，否则进程会失去管理入口。 */
  const stopRunsOf = (projectId: string) => {
    for (const run of runs) {
      if (run.projectId === projectId && run.running) {
        void stop(run.runId).catch((cause) =>
          toast.error(`停止失败：${String(cause)}`),
        );
      }
    }
  };

  /** 切项目时把它最近一次运行的输出带出来；没有记录就显示占位提示。 */
  const selectProject = (project: ProjectEntry) => {
    setActiveId(project.id);
    setPendingDelete(null);
    const latest = runs.find((run) => run.projectId === project.id);
    if (latest) void openRun(latest.runId);
    else closeRun();
  };

  /** 同一个按钮：没跑就启动，在跑就停止。 */
  const handleRun = (project: ProjectEntry, index: number) => {
    const command = project.commands[index];
    if (!command) return;
    const running = findRunning(projectCommandKey(project.id, index));
    if (running) {
      void stop(running.runId).catch((cause) =>
        toast.error(`停止失败：${String(cause)}`),
      );
      return;
    }
    void start(project, command, index).catch((cause) =>
      toast.error(`启动失败：${String(cause)}`),
    );
  };

  return (
    <div className="flex h-full min-h-0 w-full flex-col gap-4">
      <div className="flex shrink-0 flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <h1 className="font-heading text-lg font-medium">项目</h1>
          <p className="text-sm text-muted-foreground">
            左侧选择项目，右侧启动命令并实时查看输出（不弹命令行窗口）。
          </p>
        </div>
        <Button size="sm" onClick={() => setEditor(null)}>
          <Plus data-icon="inline-start" />
          添加项目
        </Button>
      </div>

      {projects.length === 0 ? (
        <div className="flex flex-1 flex-col items-center justify-center gap-3 rounded-xl border border-dashed border-border/70 px-6 py-14 text-center">
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
        <div className="flex min-h-0 flex-1 gap-4">
          {/* 左列：项目列表 */}
          <aside className="flex w-60 shrink-0 flex-col gap-1 overflow-y-auto rounded-xl p-1.5 ring-1 ring-[color:var(--frost-border)] [background-image:var(--frost-surface)]">
            {projects.map((project) => {
              const runningCount = runs.filter(
                (run) => run.projectId === project.id && run.running,
              ).length;
              const active = project.id === activeProject?.id;
              return (
                <button
                  key={project.id}
                  type="button"
                  onClick={() => selectProject(project)}
                  className={cn(
                    "flex w-full shrink-0 flex-col gap-0.5 rounded-lg px-2.5 py-2 text-left transition-colors",
                    active ? "bg-brand-soft" : "hover:bg-muted/50",
                  )}
                >
                  <span className="flex min-w-0 items-center gap-1.5">
                    <FolderGit2
                      className={cn(
                        "size-3.5 shrink-0",
                        active ? "text-brand" : "text-muted-foreground",
                      )}
                      aria-hidden="true"
                    />
                    <span className="min-w-0 flex-1 truncate text-xs font-medium">
                      {project.name}
                    </span>
                    {runningCount > 0 && (
                      <span
                        className="size-1.5 shrink-0 rounded-full bg-emerald-500"
                        title={`${runningCount} 个命令运行中`}
                      />
                    )}
                  </span>
                  <span className="truncate pl-5 text-[0.65rem] text-muted-foreground">
                    {project.path || "按各命令目录启动"}
                  </span>
                </button>
              );
            })}
          </aside>

          {/* 右列：选中项目的命令 + 实时输出 */}
          <section className="flex min-w-0 flex-1 flex-col gap-3">
            {activeProject && (
              <>
                <div className="flex shrink-0 items-center gap-2.5 rounded-xl px-3 py-2.5 ring-1 ring-[color:var(--frost-border)] [background-image:var(--frost-surface)]">
                  <span className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-brand-soft text-brand">
                    <FolderGit2 className="size-4" aria-hidden="true" />
                  </span>
                  <div className="min-w-0 flex-1">
                    <p className="truncate text-sm font-medium">{activeProject.name}</p>
                    <p
                      className="truncate text-xs text-muted-foreground"
                      title={activeProject.path || undefined}
                    >
                      {activeProject.path || "未设置项目目录，按各命令的目录启动"}
                    </p>
                  </div>

                  {activeProject.path.trim() !== "" && (
                    <Button
                      variant="outline"
                      size="xs"
                      title={`在资源管理器中打开 ${activeProject.path}`}
                      onClick={() => void openFolder(activeProject)}
                    >
                      <FolderOpen data-icon="inline-start" />
                      打开目录
                    </Button>
                  )}

                  <Button
                    variant="ghost"
                    size="icon-xs"
                    aria-label={`编辑 ${activeProject.name}`}
                    title="编辑"
                    onClick={() => setEditor(activeProject)}
                  >
                    <Pencil />
                  </Button>

                  {pendingDelete === activeProject.id ? (
                    <div className="flex shrink-0 items-center gap-1">
                      <Button
                        variant="destructive"
                        size="xs"
                        onClick={() => {
                          stopRunsOf(activeProject.id);
                          if (selectedRun?.projectId === activeProject.id) closeRun();
                          void removeProject(activeProject.id);
                          setActiveId(null);
                          setPendingDelete(null);
                          if (editor?.id === activeProject.id) setEditor(undefined);
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
                    </div>
                  ) : (
                    <Button
                      variant="ghost"
                      size="icon-xs"
                      aria-label={`删除 ${activeProject.name}`}
                      title="删除"
                      onClick={() => setPendingDelete(activeProject.id)}
                    >
                      <Trash2 />
                    </Button>
                  )}
                </div>

                <div className="flex shrink-0 flex-wrap items-center gap-1.5">
                  {activeProject.commands.map((command, index) => {
                    const key = projectCommandKey(activeProject.id, index);
                    const running = findRunning(key);
                    const latest = findLatest(key);
                    return (
                      <span key={key} className="flex items-center gap-0.5">
                        <Button
                          variant={running ? "destructive" : "secondary"}
                          size="xs"
                          title={
                            running
                              ? "停止并结束整棵进程树"
                              : command.dir
                                ? `${command.command}\n工作目录：${command.dir}`
                                : command.command
                          }
                          onClick={() => handleRun(activeProject, index)}
                        >
                          {running ? (
                            <Square data-icon="inline-start" />
                          ) : (
                            <Play data-icon="inline-start" />
                          )}
                          <span className="max-w-40 truncate">
                            {command.name || command.command}
                          </span>
                        </Button>
                        {latest && (
                          <Button
                            variant="ghost"
                            size="icon-xs"
                            title="查看输出"
                            aria-label={`查看 ${command.name || command.command} 的输出`}
                            onClick={() => void openRun(latest.runId)}
                          >
                            <Terminal />
                          </Button>
                        )}
                      </span>
                    );
                  })}

                  {activeProject.commands.length === 0 && (
                    <span className="text-[0.7rem] text-muted-foreground">
                      未配置启动命令，点右上角铅笔添加
                    </span>
                  )}
                </div>

                {consoleRun ? (
                  <ProjectConsole
                    className="min-h-0 flex-1"
                    run={consoleRun}
                    lines={selectedLines}
                    onClose={closeRun}
                    onStop={(runId) =>
                      void stop(runId).catch((cause) =>
                        toast.error(`停止失败：${String(cause)}`),
                      )
                    }
                    onClearLog={(runId) =>
                      void clearLog(runId).catch((cause) =>
                        toast.error(`清空输出失败：${String(cause)}`),
                      )
                    }
                    onClearRun={(runId) =>
                      void clearRun(runId).catch((cause) =>
                        toast.error(`删除记录失败：${String(cause)}`),
                      )
                    }
                  />
                ) : (
                  <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-2 rounded-xl border border-dashed border-border/70 px-6 text-center">
                    <span className="flex size-10 items-center justify-center rounded-xl bg-brand-soft text-brand">
                      <Terminal className="size-5" aria-hidden="true" />
                    </span>
                    <p className="text-sm font-medium">还没有运行记录</p>
                    <p className="max-w-sm text-xs text-muted-foreground">
                      点上面的命令启动，输出会实时显示在这里；停止会结束整棵进程树。
                    </p>
                  </div>
                )}
              </>
            )}
          </section>
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
