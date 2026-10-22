import { useCallback, useEffect, useState } from "react";

import { FolderOpen, Play } from "lucide-react";

import { Button } from "@/components/ui/button";
import { ipc, type ProjectCommand, type ProjectEntry } from "@/lib/ipc";
import { cn } from "@/lib/utils";

interface QuickPanelProjectsProps {
  /** 面板已展开且当前就是项目页：决定是否重新读取配置。 */
  active: boolean;
}

type Feedback = { tone: "ok" | "error"; text: string } | null;

/**
 * 快捷小窗的「项目」页：常开项目的一键启动。
 *
 * 项目列表存在主窗口的配置里，这里**每次展开时重读一次**（面板不挂 `useConfig`
 * 那份全局状态，重读比跨窗口同步更简单可靠）。提示走页内一行文字：
 * 面板一收起 toast 就跟着没了，等于没提示。
 */
export function QuickPanelProjects({ active }: QuickPanelProjectsProps) {
  const [projects, setProjects] = useState<ProjectEntry[]>([]);
  const [feedback, setFeedback] = useState<Feedback>(null);

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

  const runCommand = useCallback(async (project: ProjectEntry, command: ProjectCommand) => {
    try {
      await ipc.launchProjectCommand(project.path, command.command);
      setFeedback({
        tone: "ok",
        text: `已在新 cmd 窗口启动 ${project.name} · ${command.name || command.command}`,
      });
    } catch (cause) {
      setFeedback({ tone: "error", text: `启动失败：${String(cause)}` });
    }
  }, []);

  return (
    <div className="flex flex-col gap-2.5">
      <div className="flex flex-col gap-0.5">
        <h2 className="font-heading text-sm font-medium">项目</h2>
        <p className="text-xs text-muted-foreground">
          点击命令即在新 cmd 窗口运行；目录按钮打开资源管理器。
        </p>
      </div>

      {feedback && (
        <p
          className={cn(
            "line-clamp-2 break-words text-[11px] leading-snug",
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
        <div className="flex flex-col gap-2">
          {projects.map((project) => (
            <div
              key={project.id}
              className="flex flex-col gap-1 rounded-lg border border-border/60 bg-muted/30 px-2.5 py-2"
            >
              <span className="truncate text-xs font-medium">{project.name}</span>
              <span
                className="truncate text-[0.65rem] text-muted-foreground"
                title={project.path}
              >
                {project.path}
              </span>
              <div className="flex flex-wrap gap-1 pt-0.5">
                <Button
                  variant="outline"
                  size="xs"
                  title={`打开 ${project.path}`}
                  onClick={() => void openFolder(project)}
                >
                  <FolderOpen data-icon="inline-start" />
                  目录
                </Button>
                {project.commands.length === 0 ? (
                  <span className="self-center text-[0.7rem] text-muted-foreground">
                    未配置启动命令
                  </span>
                ) : (
                  project.commands.map((command, index) => (
                    <Button
                      key={`${command.name}-${index}`}
                      variant="secondary"
                      size="xs"
                      title={command.command}
                      onClick={() => void runCommand(project, command)}
                    >
                      <Play data-icon="inline-start" />
                      <span className="max-w-28 truncate">
                        {command.name || command.command}
                      </span>
                    </Button>
                  ))
                )}
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
