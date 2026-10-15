import { invoke } from "@tauri-apps/api/core";

export type Theme = "system" | "light" | "dark";

/** 快捷小窗可吸附的屏幕边缘（不含底部）。 */
export type DockEdge = "left" | "right" | "top";

/** 与 Rust 侧 `AppConfig` 一一对应。 */
export interface AppConfig {
  theme: Theme;
  language: string;
  closeToTray: boolean;
  lastOpened: string | null;
  quickPanel: boolean;
  /** 快捷小窗吸附的屏幕边缘。 */
  quickPanelEdge: DockEdge;
  /** 快捷小窗沿吸附边缘的偏移（left/right 为纵向 y，top 为横向 x），由灰条窗口独占写入。 */
  quickPanelY: number | null;
}

export const ipc = {
  getConfig: () => invoke<AppConfig>("get_config"),
  setConfig: (config: AppConfig) => invoke<void>("set_config", { config }),
  resetConfig: () => invoke<AppConfig>("reset_config"),
  /** 更新快捷小窗吸附的边缘与沿边偏移（拖动结束时写入）。 */
  setQuickPanelDock: (edge: DockEdge, offset: number | null) =>
    invoke<void>("set_quick_panel_dock", { edge, offset }),
  /** 一次性设置**灰条窗口**的几何（拖动每帧调用，合并两次 IPC）；展开/收起不改变灰条几何。 */
  setQuickPanelGeometry: (x: number, y: number, width: number, height: number) =>
    invoke<void>("set_quick_panel_geometry", { x, y, width, height }),
  /** 仅移动**面板窗口**（拖动灰条时逐帧跟随）；窗口尺寸恒定，任何路径都不缩放。 */
  setQuickPanelBodyPosition: (x: number, y: number) =>
    invoke<void>("set_quick_panel_body_position", { x, y }),
  /** 展开面板：定位（只移动）→ 显示 → 面板窗口播放展开动画；宽高恒为固定常量。 */
  openQuickPanelBody: (x: number, y: number, width: number, height: number, edge: DockEdge) =>
    invoke<void>("open_quick_panel_body", { x, y, width, height, edge }),
  /** 收起面板：面板窗口播放收起动画（结束后自行隐藏）。 */
  closeQuickPanelBody: (edge: DockEdge) => invoke<void>("close_quick_panel_body", { edge }),
  /** 直接隐藏面板窗口（动画结束回调与兜底路径）。 */
  hideQuickPanelBody: () => invoke<void>("hide_quick_panel_body"),
  /** 每次轮询变化的动态量（CPU/内存/网络/磁盘）。 */
  getSystemSnapshot: () => invoke<DynamicSnapshot>("get_system_snapshot"),
  /** 不随时间变化的静态信息（硬件型号/OS/主机名），进程启动时采集一次。 */
  getSystemStatics: () => invoke<SystemStatics>("get_system_statics"),
};

/** 与 Rust 侧 `DiskInfo` 一一对应。 */
export interface DiskInfo {
  name: string;
  /** 挂载点，Windows 上形如 `C:`。 */
  mountPoint: string;
  fileSystem: string;
  kind: string;
  total: number;
  available: number;
  isRemovable: boolean;
}

/** 与 Rust 侧 `GpuInfo` 一一对应。 */
export interface GpuInfo {
  name: string;
  /** 专用显存（字节），无法读取时为 0。 */
  memory: number;
}

/** 与 Rust 侧 `MemoryModuleInfo` 一一对应。 */
export interface MemoryModuleInfo {
  manufacturer: string;
  partNumber: string;
  capacity: number;
  speed: number;
  memoryType: string;
}

/** 与 Rust 侧 `SystemSnapshot` 一一对应：每次轮询变化的动态量。 */
export interface DynamicSnapshot {
  /** CPU 全局占用率（0-100），首次采集无有效差值时为 null。 */
  cpuUsage: number | null;
  memUsed: number;
  memTotal: number;
  netRxPerSec: number;
  netTxPerSec: number;
  uptimeSecs: number;
  disks: DiskInfo[];
}

/** 与 Rust 侧 `SystemStatics` 一一对应：进程启动时采集一次的静态信息。 */
export interface SystemStatics {
  hostname: string;
  osName: string;
  osLongName: string;
  osVersion: string;
  ip: string;
  /** CPU 型号。 */
  cpuBrand: string;
  /** 主板型号。 */
  motherboard: string;
  /** 整机型号。 */
  systemModel: string;
  gpus: GpuInfo[];
  memoryModules: MemoryModuleInfo[];
}

/** 静态 + 动态合并后的完整快照（页面展示用，由 `useSystemMonitor` 组装）。 */
export type SystemSnapshot = DynamicSnapshot & SystemStatics;
