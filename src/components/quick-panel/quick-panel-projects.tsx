import { useCallback, useEffect, useState } from "react";

import { FolderOpen, Play, Square, Terminal } from "lucide-react";

import { ProjectConsole } from "@/components/projects/project-console";
import { Button } from "@/components/ui/button";
import { projectCommandKey, useProjectRuns } from "@/hooks/use-project-runs";
import { ipc, type ProjectCommand, type ProjectEntry } from "@/lib/ipc";
import { cn } from "@/lib/utils";

interface QuickPanelProjectsProps {
  /** 面板已展开且当前就是项目页：决定是否重新读取配置。 */
  active: boolean;
  /**
   * 是否在列表下方内嵌命令输出。组合窗口（`hub`）开着；快捷小窗面板又窄又矮，
   * 塞进控制台后列表就没地方了，那里仍按「输出去主窗口看」。
   */
  withOutput?: boolean;
}

type Feedback = { tone: "ok" | "error"; text: string } | null;

/**
 * 快捷小窗 / 组合窗口的「项目」页：常开项目的一键启动 + （可选）就地查看输出。
 *
 * 项目列表存在主窗口的配置里，这里**每次展开时重读一次**（面板不挂 `useConfig`
 * 那份全局状态，重读比跨窗口同步更简单可靠）。提示走页内一行文字：
 * 主窗口之外没有常驻的 Toaster 语境，页内提示最稳。
 */
export function QuickPanelProjects({ active, withOutput = false }: QuickPanelProjectsProps) {
  const [projects, setProjects] = useState<ProjectEntry[]>([]);
  const [feedback, setFeedback] = useState<Feedback>(null);
  const {
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

  useEffect(() => {
    if (!active) {
      setFeedback(null);
      return;
    }
    let cancelled = false;
    void ipc
      .getConfig()
      .then((config) => {
        if (!cancelled) setProjects(config.projects);
      })
      .catch((cause) => {
        if (!cancelled) setFeedback({ tone: "error", text: `读取项目失败：${String(cause)}` });
      });
    return () => {
      cancelled = true;
    };
  }, [active]);

  const openFolder = useCallback(async (project: ProjectEntry) => {
    try {
      await ipc.openProjectFolder(project.path);
      setFeedback({ tone: "ok", text: `已在资源管理器打开 ${project.name}` });
    } catch (cause) {
      setFeedback({ tone: "error", text: `打开文件夹失败：${String(cause)}` });
    }
  }, []);

  /** 与主窗口一致：没跑就启动，在跑就停止；启动后下方自动跟着显示这条命令的输出。 */
  const runCommand = useCallback(
    async (project: ProjectEntry, command: ProjectCommand, index: number) => {
      const label = `${project.name} · ${command.name || command.command}`;
      const running = findRunning(projectCommandKey(project.id, index));
      try {
        if (running) {
          await stop(running.runId);
          setFeedback({ tone: "ok", text: `已停止 ${label}` });
        } else {
          await start(project, command, index);
          setFeedback({ tone: "ok", text: `已在应用内启动 ${label}` });
        }
      } catch (cause) {
        setFeedback({
          tone: "error",
          text: `${running ? "停止" : "启动"}失败：${String(cause)}`,
        });
      }
    },
    [findRunning, start, stop],
  );

  /** 控制台里的动作失败一律落到页内提示：这个页面不保证有 Toaster。 */
  const guard = useCallback((what: string, action: () => Promise<unknown>) => {
    action().catch((cause) => setFeedback({ tone: "error", text: `${what}失败：${String(cause)}` }));
  }, []);

  return (
    <div
      className={cn(
        "flex flex-col gap-2.5",
        // 带上输出时，列表与输出要分掉主视图的整块高度：列表短则原样、长则自己滚，
        // 输出吃掉剩下的空间（见 `hub-panel.tsx` 的 `HubMainView scroll={false}`）。
        withOutput && "h-full min-h-0",
      )}
    >
      <div className="flex shrink-0 flex-col gap-0.5">
        <h2 className="font-heading text-sm font-medium">项目</h2>
        <p className="text-xs text-muted-foreground">
          {withOutput
            ? "点击命令即在应用内启动 / 停止（不弹命令行窗口），下方实时显示所选命令的输出。"
            : "点击命令即在应用内启动 / 停止（不弹命令行窗口）；输出到主窗口「项目」页查看。"}
        </p>
      </div>

      {feedback && (
        <p
          className={cn(
            "line-clamp-2 shrink-0 break-words text-[11px] leading-snug",
            feedback.tone === "error" ? "text-destructive" : "text-muted-foreground",
          )}
        >
          {feedback.text}
        </p>
      )}

      {projects.length === 0 ? (
        <p className="rounded-lg border border-dashed border-border/70 px-3 py-3 text-xs leading-relaxed text-muted-foreground">
          还没有项目。到主窗口的「项目」页添加名称、目录和启动命令后，这里就能一键启动。
        </p>
      ) : (
        <div
          className={cn(
            "flex flex-col gap-2",
            withOutput && "min-h-0 shrink overflow-y-auto pr-0.5",
          )}
        >
          {projects.map((project) => (
            <div
              key={project.id}
              className="flex flex-col gap-1 rounded-lg border border-border/60 bg-muted/30 px-2.5 py-2"
            >
              <span className="truncate text-xs font-medium">{project.name}</span>
              <span
                className="truncate text-[0.65rem] text-muted-foreground"
                title={project.path || undefined}
              >
                {project.path || "按各命令目录启动"}
              </span>
              <div className="flex flex-wrap gap-1 pt-0.5">
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
                {project.commands.length === 0 ? (
                  <span className="self-center text-[0.7rem] text-muted-foreground">
                    未配置启动命令
                  </span>
                ) : (
                  project.commands.map((command, index) => {
                    const key = projectCommandKey(project.id, index);
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
                          onClick={() => void runCommand(project, command, index)}
                        >
                          {running ? (
                            <Square data-icon="inline-start" />
                          ) : (
                            <Play data-icon="inline-start" />
                          )}
                          <span className="max-w-28 truncate">
                            {command.name || command.command}
                          </span>
                        </Button>
                        {withOutput && latest && (
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
                  })
                )}
              </div>
            </div>
          ))}
        </div>
      )}

      {withOutput &&
        (selectedRun ? (
          <ProjectConsole
            className="min-h-48 flex-1"
            run={selectedRun}
            lines={selectedLines}
            onClose={closeRun}
            onStop={(runId) => guard("停止", () => stop(runId))}
            onClearLog={(runId) => guard("清空输出", () => clearLog(runId))}
            onClearRun={(runId) => guard("删除记录", () => clearRun(runId))}
          />
        ) : (
          <div className="flex min-h-48 shrink-0 flex-col items-center justify-center gap-2 rounded-xl border border-dashed border-border/70 px-6 text-center">
            <span className="flex size-10 items-center justify-center rounded-xl bg-brand-soft text-brand">
              <Terminal className="size-5" aria-hidden="true" />
            </span>
            <p className="text-sm font-medium">还没有运行记录</p>
            <p className="max-w-sm text-xs text-muted-foreground">
              点上面的命令启动，输出会实时显示在这里；停止会结束整棵进程树。
            </p>
          </div>
        ))}
    </div>
  );
}