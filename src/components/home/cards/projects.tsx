import { FolderOpen, Play, Square, Terminal } from "lucide-react";
import { toast } from "sonner";

import { HomeCardHint, HomeCardLink } from "@/components/home/home-card";
import { Button } from "@/components/ui/button";
import { projectCommandKey, useProjectRuns } from "@/hooks/use-project-runs";
import { useProjects } from "@/hooks/use-projects";
import { useAppStore } from "@/stores/app-store";
import type { ProjectEntry } from "@/lib/ipc";

/** 首页卡片里最多展开几个项目；更多走「全部项目」。 */
const VISIBLE_LIMIT = 4;

/** 首页「项目」卡片：常开项目 + 各自的第一条命令，一键开目录 / 起停服务。 */
export function ProjectsCard() {
  const setActiveView = useAppStore((state) => state.setActiveView);
  const openProjectRun = useAppStore((state) => state.openProjectRun);
  const { projects, openFolder } = useProjects();
  const { runs, start, stop, findRunning } = useProjectRuns();
  const visible = projects.slice(0, VISIBLE_LIMIT);

  /** 与项目页一致：没跑就启动，在跑就停止；输出到项目页看。 */
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

  if (projects.length === 0) {
    return (
      <div className="flex h-full min-h-0 flex-col gap-2">
        <HomeCardHint>
          把常开的本地服务加进来：点击即可打开目录，或由应用托管启动命令（输出在项目页看）。
        </HomeCardHint>
        <HomeCardLink onClick={() => setActiveView("projects")}>添加第一个项目</HomeCardLink>
      </div>
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col gap-2">
      <div className="flex flex-col gap-1.5">
        {visible.map((project) => (
          <div
            key={project.id}
            className="flex flex-col gap-1 rounded-lg border border-border/60 bg-muted/25 px-2 py-1.5"
          >
            <div className="flex items-center gap-1.5">
              <span className="min-w-0 truncate text-xs font-medium">{project.name}</span>
              {runs.some((run) => run.projectId === project.id && run.running) && (
                <span
                  className="size-1.5 shrink-0 rounded-full bg-emerald-500"
                  title="有命令运行中"
                />
              )}
              <span className="ml-auto shrink-0 text-[0.65rem] text-muted-foreground">
                {project.commands.length > 0
                  ? `${project.commands.length} 个命令`
                  : "仅目录"}
              </span>
            </div>
            <div className="flex flex-wrap gap-1">
              {project.path.trim() !== "" && (
                <Button
                  variant="outline"
                  size="xs"
                  title={`打开 ${project.path}`}
                  onClick={() => void openFolder(project)}
                >
                  <FolderOpen data-icon="inline-start" />
                  目录
                </Button>
              )}
              {project.commands.slice(0, 3).map((command, index) => {
                const key = projectCommandKey(project.id, index);
                const running = findRunning(key);
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
                      onClick={() => handleRun(project, index)}
                    >
                      {running ? (
                        <Square data-icon="inline-start" />
                      ) : (
                        <Play data-icon="inline-start" />
                      )}
                      <span className="max-w-20 truncate">
                        {command.name || command.command}
                      </span>
                    </Button>
                    {running && (
                      <Button
                        variant="ghost"
                        size="icon-xs"
                        title="在项目页查看运行输出"
                        aria-label={`查看 ${command.name || command.command} 的运行输出`}
                        onClick={() => openProjectRun(project.id, running.runId)}
                      >
                        <Terminal />
                      </Button>
                    )}
                  </span>
                );
              })}
            </div>
          </div>
        ))}
      </div>

      <div className="mt-auto flex items-center justify-between gap-2 pt-1">
        <span className="text-[0.65rem] text-muted-foreground tabular-nums">
          共 {projects.length} 个
        </span>
        <HomeCardLink onClick={() => setActiveView("projects")}>全部项目</HomeCardLink>
      </div>
    </div>
  );
}
