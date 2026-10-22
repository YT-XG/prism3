import { useCallback, useEffect, useMemo, useState } from "react";

import { listen } from "@tauri-apps/api/event";

import {
  ipc,
  type ProjectCommand,
  type ProjectEntry,
  type ProjectOutputLine,
  type ProjectRun,
} from "@/lib/ipc";

/** 单条运行记录在前端最多保留多少行（与 Rust 侧 `MAX_LOG_LINES` 一致）。 */
const MAX_LOG_LINES = 4000;

/** 命令按钮与运行记录的对应键；命令没有稳定 id，用「项目 id + 下标」定位。 */
export function projectCommandKey(projectId: string, index: number): string {
  return `${projectId}:${index}`;
}

/**
 * 项目命令的运行状态与实时输出（主窗口、首页卡片、快捷小窗共用）。
 *
 * 状态由 `project:runs` 整份广播，输出由 `project:output` 增量推送；
 * 只有**已经打开控制台**的运行记录才会在前端缓存输出，其余留在 Rust 侧，
 * 点「输出」时用 `get_project_run_log` 一次性回放。
 *
 * 本钩子不弹提示：快捷小窗没有 Toaster，失败交给调用方用页内文字或 toast 表达。
 */
export function useProjectRuns() {
  const [runs, setRuns] = useState<ProjectRun[]>([]);
  const [logs, setLogs] = useState<Record<string, ProjectOutputLine[]>>({});
  /** 当前打开控制台的运行记录；null = 控制台收起。 */
  const [selectedRunId, setSelectedRunId] = useState<string | null>(null);

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];

    // 事件订阅 + 挂载时兜底拉一次（事件不缓存，先订阅再拉取不会漏掉变化）。
    void listen<ProjectRun[]>("project:runs", (event) => {
      if (!disposed) setRuns(event.payload);
    }).then((unlisten) => {
      if (disposed) unlisten();
      else unlisteners.push(unlisten);
    });

    void listen<{ runId: string; lines: ProjectOutputLine[] }>(
      "project:output",
      (event) => {
        if (disposed) return;
        const { runId, lines } = event.payload;
        setLogs((prev) => {
          const current = prev[runId];
          // 没打开控制台的记录不缓存：后端缓冲里有全量，打开时再拉。
          if (!current) return prev;
          const merged = current.concat(lines);
          return {
            ...prev,
            [runId]:
              merged.length > MAX_LOG_LINES ? merged.slice(-MAX_LOG_LINES) : merged,
          };
        });
      },
    ).then((unlisten) => {
      if (disposed) unlisten();
      else unlisteners.push(unlisten);
    });

    void ipc
      .getProjectRuns()
      .then((next) => {
        if (!disposed) setRuns(next);
      })
      .catch(() => {
        // 拉取失败只影响「应用重启前的记录」这类兜底场景；后续事件仍会刷新。
      });

    return () => {
      disposed = true;
      for (const unlisten of unlisteners) unlisten();
    };
  }, []);

  /** 打开某条记录的控制台：回放后端缓冲，之后的增量由事件追加。 */
  const openRun = useCallback(async (runId: string) => {
    setSelectedRunId(runId);
    try {
      const lines = await ipc.getProjectRunLog(runId);
      setLogs((prev) => ({ ...prev, [runId]: lines }));
    } catch {
      // 回放失败不影响继续接收增量：先占一个空数组，事件仍能追加。
      setLogs((prev) => ({ ...prev, [runId]: prev[runId] ?? [] }));
    }
  }, []);

  const closeRun = useCallback(() => setSelectedRunId(null), []);

  /** 启动一条命令；失败直接抛出，由调用方决定提示方式。 */
  const start = useCallback(
    async (project: ProjectEntry, command: ProjectCommand, index: number) => {
      const key = projectCommandKey(project.id, index);
      const run = await ipc.runProjectCommand({
        key,
        projectId: project.id,
        projectName: project.name,
        commandName: command.name.trim() || command.command,
        path: project.path,
        command: command.command,
        dir: command.dir ?? "",
      });
      // 后端可能已经吐了几行：回放一次，避免「先占位再收事件」漏掉开头。
      try {
        const lines = await ipc.getProjectRunLog(run.runId);
        setLogs((prev) => ({ ...prev, [run.runId]: lines }));
      } catch {
        setLogs((prev) => ({ ...prev, [run.runId]: [] }));
      }
      setSelectedRunId(run.runId);
      return run;
    },
    [],
  );

  const stop = useCallback(async (runId: string) => {
    await ipc.stopProjectCommand(runId);
  }, []);

  const clearLog = useCallback(async (runId: string) => {
    await ipc.clearProjectRunLog(runId);
    setLogs((prev) => ({ ...prev, [runId]: [] }));
  }, []);

  /** 删除一条已结束的记录（同时丢掉本地缓存）。 */
  const clearRun = useCallback(async (runId: string) => {
    await ipc.clearProjectRun(runId);
    setLogs((prev) => {
      const next = { ...prev };
      delete next[runId];
      return next;
    });
    setSelectedRunId((current) => (current === runId ? null : current));
  }, []);

  const findRunning = useCallback(
    (key: string) => runs.find((run) => run.key === key && run.running) ?? null,
    [runs],
  );

  /** 该按钮最近一次运行记录（含已结束的，用于「查看输出」）。 */
  const findLatest = useCallback(
    (key: string) => runs.find((run) => run.key === key) ?? null,
    [runs],
  );

  const selectedRun = useMemo(
    () => runs.find((run) => run.runId === selectedRunId) ?? null,
    [runs, selectedRunId],
  );
  const selectedLines = selectedRunId ? logs[selectedRunId] ?? [] : [];

  return {
    runs,
    selectedRunId,
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
  };
}
