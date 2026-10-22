import { useCallback } from "react";

import { toast } from "sonner";

import { ipc, type ProjectEntry } from "@/lib/ipc";
import { useAppStore } from "@/stores/app-store";

/** 项目没有天然唯一键，id 只用于增删改定位。 */
function createProjectId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return crypto.randomUUID();
  }
  return `project-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

/** 去掉输入框里的首尾空白；命令行两栏都空的直接丢弃。 */
export function normalizeProject(entry: ProjectEntry): ProjectEntry {
  return {
    id: entry.id,
    name: entry.name.trim(),
    path: entry.path.trim(),
    commands: entry.commands
      .map((command) => ({
        name: command.name.trim(),
        command: command.command.trim(),
        dir: command.dir?.trim() ?? "",
      }))
      .filter((command) => command.name !== "" || command.command !== ""),
  };
}

/**
 * 主窗口的项目列表操作。
 *
 * 项目列表存在 `AppConfig.projects`，整份走 `saveConfig`（与首页布局同一路径）；
 * 命令的启动 / 停止与输出见 `use-project-runs.ts`。
 */
export function useProjects() {
  const config = useAppStore((state) => state.config);
  const saveConfig = useAppStore((state) => state.saveConfig);
  const projects = config?.projects ?? [];

  const persist = useCallback(
    async (next: ProjectEntry[]) => {
      try {
        await saveConfig({ projects: next.map(normalizeProject) });
      } catch (cause) {
        toast.error(`保存项目失败：${String(cause)}`);
        throw cause;
      }
    },
    [saveConfig],
  );

  const addProject = useCallback(
    (entry: Omit<ProjectEntry, "id">) =>
      persist([...projects, { ...entry, id: createProjectId() }]),
    [persist, projects],
  );

  const updateProject = useCallback(
    (entry: ProjectEntry) =>
      persist(projects.map((item) => (item.id === entry.id ? entry : item))),
    [persist, projects],
  );

  const removeProject = useCallback(
    (id: string) => persist(projects.filter((item) => item.id !== id)),
    [persist, projects],
  );

  const openFolder = useCallback(async (project: ProjectEntry) => {
    try {
      await ipc.openProjectFolder(project.path);
    } catch (cause) {
      toast.error(`打开文件夹失败：${String(cause)}`);
    }
  }, []);

  return {
    projects,
    addProject,
    updateProject,
    removeProject,
    openFolder,
  };
}
