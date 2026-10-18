import { useCallback, useEffect, useRef, useState } from "react";

import { listen } from "@tauri-apps/api/event";
import { error as logError } from "@tauri-apps/plugin-log";
import { toast } from "sonner";

import {
  ipc,
  type AncGradient,
  type BleCandidate,
  type BleState,
  type EqPreset,
  type NoiseMode,
} from "@/lib/ipc";

/** 耳机状态广播事件名，与 Rust 侧 `BLE_STATE_EVENT` 一致。 */
export const BLE_STATE_EVENT = "ble:state";

/**
 * 蓝牙耳机状态订阅与控制：主窗口「蓝牙」页与快捷小窗「蓝牙」页共用的数据源。
 *
 * 先订阅 `ble:state`（后续变化），再 `get_ble_state` 兜底拉一次——事件不缓存，
 * 窗口挂载 / 热重载时可能已错过最后一次广播（沿用 `useAppStatus` 的做法）。
 * 兜底拉取按可见性 gating：窗口隐藏时不发 IPC，重新可见时再拉一次补上。
 *
 * 控制动作只负责发起命令：设备是否接受由后端广播的 `phase` / `error` 决定，
 * 因此前端不做乐观更新，失败弹 `toast.error` 而不抛出（连接被厂商 App
 * 占用是正常结果，不该冒成未捕获拒绝）。
 *
 * 设备列表（增删 / 选中 / 自动连接）走专用命令，前端**既不读也不写**配置里的 BLE 三项：
 * 后端是权威的读改写方，界面一律渲染 `ble:state` 快照，这样主窗口那份可能过期的配置
 * 副本不会反过来覆盖掉快捷小窗刚做的改动。
 */
export function useEarbuds() {
  const [state, setState] = useState<BleState | null>(null);
  const [candidates, setCandidates] = useState<BleCandidate[]>([]);
  const [scanning, setScanning] = useState(false);
  const loggedErrorRef = useRef(false);

  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | undefined;

    const pull = () => {
      void ipc
        .getBleState()
        .then((next) => {
          if (active) setState(next);
        })
        .catch((cause) => {
          // 失败只记一次，避免反复刷日志。
          if (loggedErrorRef.current) return;
          loggedErrorRef.current = true;
          void logError(`读取耳机状态失败：${String(cause)}`);
        });
    };

    void listen<BleState>(BLE_STATE_EVENT, (event) => {
      if (active) setState(event.payload);
    })
      .then((off) => {
        if (active) unlisten = off;
        else off();
      })
      .catch((cause) => {
        void logError(`订阅耳机状态失败：${String(cause)}`);
      });

    const handleVisibility = () => {
      if (document.visibilityState === "visible") pull();
    };

    if (document.visibilityState === "visible") pull();
    document.addEventListener("visibilitychange", handleVisibility);

    return () => {
      active = false;
      unlisten?.();
      document.removeEventListener("visibilitychange", handleVisibility);
    };
  }, []);

  /** 动作的统一失败出口：只弹提示，不让异常冒到 `void handler()` 变成未捕获拒绝。 */
  const run = useCallback(async (what: string, action: () => Promise<unknown>) => {
    try {
      await action();
    } catch (cause) {
      toast.error(`${what}失败：${String(cause)}`);
    }
  }, []);

  const connect = useCallback(() => run("连接", () => ipc.connectEarbuds()), [run]);

  const disconnect = useCallback(() => run("断开", () => ipc.disconnectEarbuds()), [run]);

  const refresh = useCallback(() => run("刷新", () => ipc.refreshEarbudState()), [run]);

  const readBattery = useCallback(() => run("读取电量", () => ipc.readEarbudBattery()), [run]);

  const setNoiseMode = useCallback(
    (mode: NoiseMode) => run("切换降噪模式", () => ipc.setNoiseMode(mode)),
    [run],
  );

  const setAncGradient = useCallback(
    (level: AncGradient) => run("设置降噪梯度", () => ipc.setAncGradient(level)),
    [run],
  );

  const setAdaptiveAnc = useCallback(
    (enabled: boolean) => run("设置自适应降噪", () => ipc.setAdaptiveAnc(enabled)),
    [run],
  );

  const setEqPreset = useCallback(
    (preset: EqPreset) => run("切换音效", () => ipc.setEqPreset(preset)),
    [run],
  );

  const setGameMode = useCallback(
    (enabled: boolean) => run("设置游戏模式", () => ipc.setGameMode(enabled)),
    [run],
  );

  const setDualDevice = useCallback(
    (enabled: boolean) => run("设置双设备连接", () => ipc.setDualDevice(enabled)),
    [run],
  );

  /** 扫描一次附近的 BLE 设备：结果留在 `candidates`，由界面自行决定添加哪一台。 */
  const scan = useCallback(
    () =>
      run("扫描设备", async () => {
        setScanning(true);
        try {
          setCandidates(await ipc.scanBleDevices());
        } finally {
          setScanning(false);
        }
      }),
    [run],
  );

  /** 把某台设备设为「当前设备」（自动连接的目标）。 */
  const setActive = useCallback(
    (address: string) => run("切换当前设备", () => ipc.setBleActiveDevice(address)),
    [run],
  );

  /** 开关「开仓取出耳机时自动连上」。 */
  const setAutoConnect = useCallback(
    (enabled: boolean) => run("设置自动连接", () => ipc.setBleAutoConnect(enabled)),
    [run],
  );

  /** 把扫描到的一台设备加进列表（后端按地址去重）并设为当前设备。 */
  const addDevice = useCallback(
    (candidate: BleCandidate) =>
      run("添加设备", async () => {
        await ipc.addBleDevice(candidate.address, candidate.name);
        await ipc.setBleActiveDevice(candidate.address);
      }),
    [run],
  );

  /** 从列表里移除一台设备；被移除的正好是当前设备时由后端自行处理选中项。 */
  const removeDevice = useCallback(
    (address: string) => run("移除设备", () => ipc.removeBleDevice(address)),
    [run],
  );

  return {
    state,
    connect,
    disconnect,
    refresh,
    readBattery,
    setNoiseMode,
    setAncGradient,
    setAdaptiveAnc,
    setEqPreset,
    setGameMode,
    setDualDevice,
    scan,
    candidates,
    scanning,
    setActive,
    setAutoConnect,
    addDevice,
    removeDevice,
  };
}
