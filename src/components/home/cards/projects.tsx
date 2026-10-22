import { FolderOpen, Play } from "lucide-react";

import { HomeCardHint, HomeCardLink } from "@/components/home/home-card";
import { Button } from "@/components/ui/button";
import { useProjects } from "@/hooks/use-projects";
import { useAppStore } from "@/stores/app-store";

/** 首页卡片里最多展开几个项目；更多走「全部项目」。 */
const VISIBLE_LIMIT = 4;

/** 首页「项目」卡片：常开项目 + 各自的第一条命令，一键开目录 / 起服务。 */
export function ProjectsCard() {
  const setActiveView = useAppStore((state) => state.setActiveView);
  const { projects, openFolder, runCommand } = useProjects();
  const visible = projects.slice(0, VISIBLE_LIMIT);

  if (projects.length === 0) {
    return (
      <div className="flex h-full min-h-0 flex-col gap-2">
        <HomeCardHint>
          把常开的本地服务加进来，点击即可打开目录或在新 cmd 窗口里跑启动命令。
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
            <div className="flex items-baseline gap-1.5">
              <span className="min-w-0 truncate text-xs font-medium">{project.name}</span>
              <span className="ml-auto shrink-0 text-[0.65rem] text-muted-foreground">
                {project.commands.length > 0
                  ? `${project.commands.length} 个命令`
                  : "仅目录"}
              </span>
            </div>
            <div className="flex flex-wrap gap-1">
              <Button
                variant="outline"
                size="xs"
                title={`打开 ${project.path}`}
                onClick={() => void openFolder(project)}
              >
                <FolderOpen data-icon="inline-start" />
                目录
              </Button>
              {project.commands.slice(0, 3).map((command, index) => (
                <Button
                  key={`${command.name}-${index}`}
                  variant="secondary"
                  size="xs"
                  title={command.command}
                  onClick={() => void runCommand(project, command)}
                >
                  <Play data-icon="inline-start" />
                  <span className="max-w-24 truncate">{command.name || command.command}</span>
                </Button>
              ))}
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
