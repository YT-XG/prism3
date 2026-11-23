import { useCallback, useEffect, useRef, useState, type RefObject } from "react";

import { ipc, type PhoneStatus, type WirelessInfo } from "@/lib/ipc";

/**
 * 无线调试（adb over Wi-Fi）的状态与动作。
 *
 * 只给对话框用：进对话框时拉一次，之后每次动作结束都用后端返回的新状态覆盖
 * （后端那一步已经做完 `adb connect` 的等待，这里不用再猜）。
 */
export function usePhoneWireless() {
  const [info, setInfo] = useState<WirelessInfo | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setInfo(await ipc.phoneWirelessInfo());
    } catch {
      // 只读探测失败不值得弹错：界面按「还没有信息」显示即可。
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  /** 统一跑一个动作：置 busy、拿新状态、失败时把原因抛给调用方去 toast。 */
  const run = useCallback(async (label: string, action: () => Promise<WirelessInfo>) => {
    setBusy(label);
    try {
      const next = await action();
      setInfo(next);
      return next;
    } finally {
      setBusy(null);
    }
  }, []);

  return {
    info,
    busy,
    refresh,
    /** 一键切无线（adbd → TCP 5555 → connect）。 */
    enable: () => run("enable", () => ipc.phoneWirelessEnable()),
    /** 自动发现并连接（记住的地址 → mDNS 广播 → 局域网邻居表）。 */
    discover: () => run("discover", () => ipc.phoneWirelessDiscover()),
    connect: (address: string) => run("connect", () => ipc.phoneWirelessConnect(address)),
    disconnect: (address: string) =>
      run("disconnect", () => ipc.phoneWirelessDisconnect(address)),
    /** 配对走的是另一条 `adb pair`，成功后再手动连接。 */
    pair: async (address: string, code: string) => {
      setBusy("pair");
      try {
        await ipc.phoneWirelessPair(address, code);
        await refresh();
      } finally {
        setBusy(null);
      }
    },
  };
}

/** 设备分辨率的像素尺寸（= 一帧截图的原始尺寸，坐标换算也以它为准）。 */
export interface PhoneFrameSize {
  width: number;
  height: number;
}

/**
 * adb / 设备探测。
 *
 * 只做「读环境」与两个动作（下载 adb、记住选中的设备），不碰镜像循环——
 * 镜像有自己的生命周期（开关、可见性），混在一起会让卸载清理变得难写。
 */
export function usePhone() {
  const [status, setStatus] = useState<PhoneStatus | null>(null);
  const [probing, setProbing] = useState(true);
  const [installing, setInstalling] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const probe = useCallback(async () => {
    setProbing(true);
    try {
      const next = await ipc.phoneStatus();
      setStatus(next);
      setError(next.error);
    } catch (cause) {
      setError(String(cause));
    } finally {
      setProbing(false);
    }
  }, []);

  useEffect(() => {
    void probe();
  }, [probe]);

  /** 下载 platform-tools 并重新探测；返回是否成功（页面据此决定 toast 文案）。 */
  const install = useCallback(async () => {
    setInstalling(true);
    setError(null);
    try {
      const next = await ipc.phoneInstallAdb();
      setStatus(next);
      setError(next.error);
      return true;
    } catch (cause) {
      setError(String(cause));
      return false;
    } finally {
      setInstalling(false);
    }
  }, []);

  /** 选中一台设备：先就地更新（界面立刻响应），再把选择写进配置。 */
  const select = useCallback(async (serial: string) => {
    setStatus((previous) => (previous ? { ...previous, selected: serial } : previous));
    try {
      await ipc.phoneSelectDevice(serial);
    } catch (cause) {
      // 记不住上次选的设备不影响本次使用，只记日志级别的提示。
      console.warn("记住手机设备失败：", cause);
    }
  }, []);

  return { status, probing, installing, error, probe, install, select };
}

/**
 * 镜像循环：**一帧接一帧**地拉 `screencap`，画到 canvas 上。
 *
 * 用 canvas 而不是每帧换一个 `<img src=blob:>`：后者要不停 `createObjectURL` /
 * `revokeObjectURL`，回收早一帧就会闪一下，而且还得靠 `img.onload` 纠正时序。
 * canvas 是直接覆盖绘制，不存在「新旧帧交替」的那一帧。
 *
 * 抽成 hook 而不是写进组件，是为了让「开/关」「换设备」「卸载」的清理集中在一处：
 * 任何一次清理漏掉 `cancelled`，都会留下一个永远在截屏的循环。
 */
export function usePhoneMirror({
  canvasRef,
  serial,
  enabled,
}: {
  canvasRef: RefObject<HTMLCanvasElement | null>;
  serial: string | null;
  enabled: boolean;
}) {
  const [size, setSize] = useState<PhoneFrameSize | null>(null);
  const [fps, setFps] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const sizeRef = useRef<PhoneFrameSize | null>(null);

  useEffect(() => {
    if (!serial || !enabled) {
      setFps(0);
      return;
    }

    let cancelled = false;
    let timer: number | undefined;
    let last = 0;
    const stamps: number[] = [];

    const loop = async () => {
      if (cancelled) return;
      let delay = 30;

      // 窗口被最小化 / 被隐藏时不抓帧：手机每帧都要真的截一次屏，
      // 看不见的时候继续拉只是白白耗电。
      if (typeof document !== "undefined" && document.hidden) {
        setFps(0);
        timer = window.setTimeout(() => void loop(), 800);
        return;
      }

      try {
        const buffer = await ipc.phoneScreencap(serial);
        if (cancelled) return;

        const bitmap = await createImageBitmap(new Blob([buffer], { type: "image/png" }));
        if (cancelled) {
          bitmap.close();
          return;
        }
        const canvas = canvasRef.current;
        if (canvas) {
          if (canvas.width !== bitmap.width || canvas.height !== bitmap.height) {
            canvas.width = bitmap.width;
            canvas.height = bitmap.height;
          }
          canvas.getContext("2d")?.drawImage(bitmap, 0, 0);
        }
        const next = { width: bitmap.width, height: bitmap.height };
        bitmap.close();

        if (sizeRef.current?.width !== next.width || sizeRef.current?.height !== next.height) {
          sizeRef.current = next;
          setSize(next);
        }
        setError(null);

        const now = performance.now();
        if (last > 0) {
          stamps.push(now - last);
          while (stamps.length > 8) stamps.shift();
          const average = stamps.reduce((sum, value) => sum + value, 0) / stamps.length;
          if (average > 0) setFps(Math.round(1000 / average));
        }
        last = now;
      } catch (cause) {
        if (cancelled) return;
        setError(String(cause));
        // 设备掉线 / 正在锁屏这类瞬时失败会自己恢复，放慢节奏继续试。
        delay = 1200;
      } finally {
        if (!cancelled) timer = window.setTimeout(() => void loop(), delay);
      }
    };

    void loop();
    return () => {
      cancelled = true;
      if (timer !== undefined) window.clearTimeout(timer);
    };
  }, [canvasRef, enabled, serial]);

  return { size, fps, error };
}
