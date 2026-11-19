import { useCallback, useEffect, useState } from "react";

import { listen } from "@tauri-apps/api/event";
import { error as logError } from "@tauri-apps/plugin-log";

import { ipc, type WorkLog, type WorkLogsChanged, type WorkProject } from "@/lib/ipc";

/** 工作记录变化的事件名，与 Rust `commands::WORKLOGS_EVENT` 一致。 */
export const WORKLOGS_EVENT = "worklogs:changed";

/**
 * 工作记录变更订阅（主窗口工作页 / 首页卡片 / 快捷小窗记录页共用）。
 *
 * 事件不缓存，窗口挂载或热重载时可能已错过最后一次广播，因此各数据 hook
 * 都会在挂载时先拉一次（revision 只负责「之后」的变化）。
 */
export function useWorkRevision(): number {
  const [revision, setRevision] = useState(0);

  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | undefined;

    void listen<WorkLogsChanged>(WORKLOGS_EVENT, (event) => {
      if (active) setRevision(event.payload.revision);
    })
      .then((off) => {
        if (active) unlisten = off;
        else off();
      })
      .catch((cause) => {
        void logError(`订阅工作记录变化失败：${String(cause)}`);
      });

    return () => {
      active = false;
      unlisten?.();
    };
  }, []);

  return revision;
}

/** 未归档的工作项目列表；随 `worklogs:changed` 重拉。 */
export function useWorkProjects(): { projects: WorkProject[]; reload: () => void } {
  const revision = useWorkRevision();
  const [projects, setProjects] = useState<WorkProject[]>([]);
  const [tick, setTick] = useState(0);

  useEffect(() => {
    let active = true;
    void ipc
      .listWorkProjects()
      .then((list) => {
        if (active) setProjects(list);
      })
      .catch((cause) => {
        void logError(`读取工作项目失败：${String(cause)}`);
      });
    return () => {
      active = false;
    };
  }, [revision, tick]);

  const reload = useCallback(() => setTick((value) => value + 1), []);
  return { projects, reload };
}

/**
 * `[from, to]`（秒级时间戳）内的**已完成**记录，最新在前。
 *
 * 时间范围变化与 `worklogs:changed` 都会触发重拉；`reload` 供保存后立即刷新。
 */
export function useWorkLogs(from: number, to: number): { logs: WorkLog[]; reload: () => void } {
  const revision = useWorkRevision();
  const [logs, setLogs] = useState<WorkLog[]>([]);
  const [tick, setTick] = useState(0);

  useEffect(() => {
    let active = true;
    void ipc
      .listWorkLogs(from, to)
      .then((list) => {
        if (active) setLogs(list);
      })
      .catch((cause) => {
        void logError(`读取工作记录失败：${String(cause)}`);
      });
    return () => {
      active = false;
    };
  }, [from, to, revision, tick]);

  const reload = useCallback(() => setTick((value) => value + 1), []);
  return { logs, reload };
}

/** 全部「未来计划」，按计划日期正序；随 `worklogs:changed` 重拉。 */
export function useWorkPlans(): { plans: WorkLog[]; reload: () => void } {
  const revision = useWorkRevision();
  const [plans, setPlans] = useState<WorkLog[]>([]);
  const [tick, setTick] = useState(0);

  useEffect(() => {
    let active = true;
    void ipc
      .listWorkPlans()
      .then((list) => {
        if (active) setPlans(list);
      })
      .catch((cause) => {
        void logError(`读取未来计划失败：${String(cause)}`);
      });
    return () => {
      active = false;
    };
  }, [revision, tick]);

  const reload = useCallback(() => setTick((value) => value + 1), []);
  return { plans, reload };
}
