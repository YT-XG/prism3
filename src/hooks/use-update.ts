import { useEffect, useRef, useState } from "react";

import { listen } from "@tauri-apps/api/event";
import { error as logError } from "@tauri-apps/plugin-log";
import { toast } from "sonner";

import { ipc, type UpdateState, type UpdateStatus } from "@/lib/ipc";
import { useAppStore } from "@/stores/app-store";

/** 更新状态广播事件名，与 Rust `updater::UPDATE_EVENT` 一致。 */
export const UPDATE_EVENT = "update:state";

/**
 * 自动更新状态与动作（设置页「软件更新」卡片使用）。
 *
 * 先 `listen` 再 `getUpdateState()` 兜底——事件不缓存，窗口挂载 / 热重载时可能已错过
 * 最后一次广播（沿用 `useEarbuds` / `useFileIndexState` 的做法）。动作只「发起命令」，
 * 界面显示的状态一律以后端广播 / 返回值为准，**不做乐观状态机**（下载进度、失败原因、
 * 是否已下载都只有 Rust 侧知道）。
 */
export function useUpdate() {
  const [state, setState] = useState<UpdateState | null>(null);

  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | undefined;

    void (async () => {
      const off = await listen<UpdateState>(UPDATE_EVENT, (event) => {
        if (active) setState(event.payload);
      });
      if (!active) {
        off();
        return;
      }
      unlisten = off;

      const current = await ipc.getUpdateState();
      if (active) setState(current);
    })().catch((cause) => {
      void logError(`读取更新状态失败：${String(cause)}`);
    });

    return () => {
      active = false;
      unlisten?.();
    };
  }, []);

  /** 统一处理命令失败：提示并返回 false（成功时返回 true，安装成功则应用会直接退出）。 */
  const run = async (action: () => Promise<UpdateState | void>): Promise<boolean> => {
    try {
      const next = await action();
      if (next) setState(next);
      return true;
    } catch (cause) {
      toast.error(`更新操作失败：${String(cause)}`);
      return false;
    }
  };

  return {
    state,
    check: () => run(() => ipc.checkUpdate()),
    reinstall: () => run(() => ipc.reinstallLatest()),
    pause: () => run(() => ipc.pauseUpdate()),
    resume: () => run(() => ipc.resumeUpdate()),
    cancel: () => run(() => ipc.cancelUpdate()),
    install: () => run(() => ipc.installUpdate()),
    openInstaller: () => run(() => ipc.openUpdateInstaller()),
    openDownloadUrl: () => run(() => ipc.openUpdateDownloadUrl()),
  };
}

/**
 * 全局更新提示（只在 App 里挂一次）。
 *
 * 启动 3s 后的静默检查发生在设置页可能还没打开的时候，所以「发现新版本 / 更新就绪」
 * 必须在全局层提示；**错误不弹全局 toast**（离线启动不该打扰用户），错误详情在设置页
 * 卡片里就地显示。挂载时只记录当前状态，不为已经发生的状态补弹提示。
 */
export function useUpdateNotifications() {
  const setActiveView = useAppStore((state) => state.setActiveView);
  const lastStatus = useRef<UpdateStatus | null>(null);

  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | undefined;

    const handle = (next: UpdateState) => {
      const previous = lastStatus.current;
      lastStatus.current = next.status;
      if (previous === next.status) return;

      if (next.status === "available" && next.version) {
        toast.info(`发现新版本 v${next.version}，正在后台下载…`, { duration: 5000 });
      } else if (next.status === "downloaded" && next.version) {
        toast.success(`更新已就绪：v${next.version}`, {
          duration: 8000,
          action: {
            label: "去安装",
            onClick: () => setActiveView("settings"),
          },
        });
      }
    };

    void (async () => {
      const off = await listen<UpdateState>(UPDATE_EVENT, (event) => {
        if (active) handle(event.payload);
      });
      if (!active) {
        off();
        return;
      }
      unlisten = off;

      const current = await ipc.getUpdateState();
      if (active) lastStatus.current = current.status;
    })().catch((cause) => {
      void logError(`订阅更新状态失败：${String(cause)}`);
    });

    return () => {
      active = false;
      unlisten?.();
    };
  }, [setActiveView]);
}
