import { invoke } from "@tauri-apps/api/core";

export type Theme = "system" | "light" | "dark";

/** 快捷小窗可吸附的屏幕边缘（不含底部）。 */
export type DockEdge = "left" | "right" | "top";

/** 应用运行状态（与 Rust 侧 `AppStatus` 一一对应）：灰条据此决定色相与动画。 */
export type AppStatus = "normal" | "busy" | "warn";

/** 与 Rust 侧 `AppConfig` 一一对应。 */
export interface AppConfig {
  theme: Theme;
  /** 配色预设 id（见 `THEME_PRESETS`）；未知值由前端回退到默认。 */
  themePreset: string;
  /** 强调色 `#rrggbb`；空串表示用默认强调色。 */
  accentColor: string;
  /** 壁纸图片绝对路径；空串 = 不用壁纸。 */
  wallpaper: string;
  /** 壁纸模糊半径（px）。 */
  wallpaperBlur: number;
  /** 壁纸蒙版强度（百分比）。 */
  wallpaperVeil: number;
  /** 卡片玻璃不透明度（百分比，越小越透）。 */
  glassOpacity: number;
  language: string;
  closeToTray: boolean;
  /** 开机时自动启动（默认开启；开发模式下后端不注册，见 `src-tauri/src/autostart.rs`）。 */
  autoStart: boolean;
  lastOpened: string | null;
  quickPanel: boolean;
  /** 快捷小窗吸附的屏幕边缘。 */
  quickPanelEdge: DockEdge;
  /** 快捷小窗沿吸附边缘的偏移（left/right 为纵向 y，top 为横向 x），由灰条窗口独占写入。 */
  quickPanelY: number | null;
  /** 是否启用本地文件搜索（自建文件名索引）。 */
  fileSearch: boolean;
  /** 文件搜索的索引根目录；空数组表示使用桌面 / 文档 / 下载 / 图片。 */
  fileSearchRoots: string[];
  /** 是否在耳机出现时自动连接（开仓取出耳机即连上）。 */
  bleAutoConnect: boolean;
  /** 自动连接的耳机地址（`bleDevices` 中选中的那一台）；null 表示未选中。 */
  bleActiveAddress: string | null;
  /** 已记住的耳机列表；增删与选中都整份写回配置，由 Rust 侧检测变化后重同步会话。 */
  bleDevices: BleDeviceEntry[];
  /** 是否记录剪贴板历史（文本 + 图片）。 */
  clipboardHistory: boolean;
  /** 剪贴板历史保留天数；`0` 表示永久保留。 */
  clipboardRetentionDays: number;
  /** 是否把剪贴板里的图片也计入历史（截图体积较大，可单独关掉）。 */
  clipboardCaptureImages: boolean;
  /** 点击一条记录时是否自动「回到上一个焦点并粘贴」；关闭后只复制到剪贴板。 */
  clipboardPasteOnClick: boolean;
  /**
   * 首页 Bento 布局（前端 `serializeHomeLayout` 生成的 JSON 字符串）。
   * 空串 = 默认布局；后端只当不透明字符串存取，解析与校验都在前端。
   */
  homeLayout: string;
  /** 速记编辑器模式（见 `NoteEditorMode`）。 */
  noteEditorMode: NoteEditorMode;
  /** 速记回收站保留天数；`0` 表示永久保留。 */
  noteTrashRetentionDays: number;
  /** 本地项目快捷启动列表（对应 Rust 侧 `ProjectEntry`）。 */
  projects: ProjectEntry[];
}

/** 一个项目的启动命令预设：短名 + 完整命令行。 */
export interface ProjectCommand {
  name: string;
  command: string;
}

/** 一个本地项目：目录 + 可一键执行的启动命令预设（命令可以为空 = 只打开文件夹）。 */
export interface ProjectEntry {
  /** 前端生成的定位 id（项目没有天然唯一键，路径允许重复）。 */
  id: string;
  name: string;
  /** 项目目录绝对路径。 */
  path: string;
  commands: ProjectCommand[];
}

/** 速记编辑器模式：实时预览 / 分屏 / 源码 / 仅预览（与 Rust 侧配置字符串一一对应）。 */
export type NoteEditorMode = "wysiwyg" | "split" | "source" | "preview";

/** 与 Rust 侧 `Note` 一一对应。时间字段为**秒级 unix 时间戳**。 */
export interface Note {
  id: number;
  title: string;
  content: string;
  createdAt: number;
  updatedAt: number;
  /** 所在文件夹（null = 树根）。 */
  folderId: number | null;
  /** 剪藏来源 URL（普通笔记为空串）。 */
  sourceUrl: string;
  /** 回收站标记（null = 正常）。 */
  deletedAt: number | null;
  /** 自定义树图标（emoji；null = 默认文件图标）。 */
  icon: string | null;
}

/** 与 Rust 侧 `NoteFolder` 一一对应。 */
export interface NoteFolder {
  id: number;
  name: string;
  parentId: number | null;
  sortOrder: number;
  /** 内置「剪藏」文件夹不可改名 / 删除。 */
  builtin: boolean;
  createdAt: number;
}

/** 与 Rust 侧 `Tag` 一一对应。 */
export interface Tag {
  id: number;
  name: string;
  createdAt: number;
  /** 内置「剪藏」标签不可改名 / 删除。 */
  builtin: boolean;
}

/** 笔记-标签关联对（前端构建 `tagId → noteIds` 筛选映射）。 */
export interface NoteTagRow {
  noteId: number;
  tagId: number;
}

/** 双链出链项（`toNoteId` 为 null 表示「未链接提及」）。 */
export interface NoteLinkOut {
  toTitle: string;
  toNoteId: number | null;
}

/** 双链入链项。 */
export interface NoteLinkIn {
  fromNoteId: number;
  fromTitle: string;
}

/** 反链面板数据。 */
export interface NoteLinks {
  outgoing: NoteLinkOut[];
  incoming: NoteLinkIn[];
}

/** 回收站清理报告。 */
export interface PurgeReport {
  purged: number;
}

/** 孤儿图片清理报告。 */
export interface NoteImageGcReport {
  dryRun: boolean;
  totalFiles: number;
  referenced: number;
  orphanFiles: string[];
  removed: number;
  failed: number;
}

/** 速记导出报告。 */
export interface ExportReport {
  exported: number;
  /** 复制进 `assets/` 的图片数。 */
  images: number;
  failed: string[];
  dir: string;
}

/** 速记导入报告。 */
export interface ImportReport {
  imported: number;
  /** `sourceUrl` 命中已有笔记而跳过（去重）。 */
  skipped: number;
  failed: string[];
  cancelled: boolean;
}

/** `notes:changed` 事件载荷（只带 revision，列表由前端据此重拉）。 */
export interface NotesChanged {
  revision: number;
}

/** 打开搜索结果的动作。 */
export type OpenAction = "open" | "reveal";

/** 文件索引所处阶段（与 Rust 侧 `IndexPhase` 一一对应）。 */
export type IndexPhase = "idle" | "loading" | "scanning" | "ready" | "error";

/** 与 Rust 侧 `IndexState` 一一对应：命令返回值与 `search:index-state` 事件载荷同构。 */
export interface FileIndexState {
  enabled: boolean;
  phase: IndexPhase;
  /** 本轮扫描已遍历的条目数。 */
  scanned: number;
  /** 索引中的条目数。 */
  indexed: number;
  /** 索引建成时间（秒），从未建成时为 null。 */
  builtAtSecs: number | null;
  /** 是否有一次刷新正在进行（结果可能略旧）。 */
  stale: boolean;
  /** 是否因达到条目上限提前停止。 */
  truncated: boolean;
  error: string | null;
  roots: string[];
  missingRoots: string[];
}

/** 与 Rust 侧 `SearchHit` 一一对应。 */
export interface SearchHit {
  path: string;
  name: string;
  /** 所在目录。 */
  parent: string;
  isDir: boolean;
  size: number;
  modifiedSecs: number;
}

/** 与 Rust 侧 `NoiseMode` 一一对应。 */
export type NoiseMode = "anc" | "normal" | "wind" | "transparency";

/** 与 Rust 侧 `AncGradient` 一一对应：降噪深度分档。 */
export type AncGradient = "light" | "medium" | "deep";

/** 与 Rust 侧 `EqPreset` 一一对应。 */
export type EqPreset = "hifi" | "pop" | "rock";

/**
 * 与 Rust 侧 `BatteryReading` 一一对应；设备报告不可用（0xFF）的字段为 null。
 *
 * 充电标志只覆盖左右耳（数据字节 5 / 6）；字节 8 的含义尚未确认，因此没有充电仓字段。
 */
export interface BatteryReading {
  left: number | null;
  right: number | null;
  case: number | null;
  /** 左耳是否正在充电；null 表示未知。 */
  leftCharging: boolean | null;
  /** 右耳是否正在充电；null 表示未知。 */
  rightCharging: boolean | null;
  /** 读取时间（秒级时间戳）。 */
  readAtSecs: number;
}

/**
 * `ble:state.devices` 的一项——对应 Rust 的 `BleDeviceEntrySerialized`。
 *
 * 注意它**不含** `adaptiveAnc`：自适应降噪是**本地记忆**，只存在于配置里，
 * 快照只在顶层给出「当前设备」的那一份（见 `BleState.adaptiveAnc`）。
 *
 * 耳机广播不带名字，`name` 为 null 是正常现象——展示统一交给 `displayDeviceName`。
 */
export interface BleDeviceSnapshotEntry {
  address: string;
  name: string | null;
  /** 最后一次连上它的时间（秒级时间戳）；从未连上为 null。 */
  lastConnectedAt: number | null;
}

/** 配置项 `bleDevices` 的一项——对应 Rust 的 `BleDeviceEntry`：比快照项多一个本地记忆。 */
export interface BleDeviceEntry extends BleDeviceSnapshotEntry {
  /**
   * 「自适应降噪」的**本地记忆**（每台设备各一份）。
   *
   * 设备从不上报这一位（`06/02` 永远读回「固定降噪」，且写后必补读），所以它只能是
   * 本机记住的偏好——官方 App 同样如此。界面请显示 `BleState.adaptiveAnc`，不要逐条读它。
   */
  adaptiveAnc: boolean;
}

/** 与 Rust 侧扫描结果一一对应：`scan_ble_devices` 的返回项。 */
export interface BleCandidate {
  address: string;
  name: string | null;
  /** 信号强度（dBm，越接近 0 越强）。 */
  rssi: number;
  /**
   * 厂商数据的十六进制字符串（原样字节，如 `00030001000b016400…`）；用于展示与厂商过滤。
   *
   * **不要拿它的尾部当地址**：早年以为末 6 字节等于设备地址，已被真机否证（同一副耳机、
   * 广播地址不变，而该尾部在 `120203047c0f` 与 `a1020306cafd` 之间跨会话变过）。
   * 身份只认 `address`。
   */
  manufacturerPayload: string;
  /** 是否已在本机记住的设备列表里。 */
  known: boolean;
}

/** 与 Rust 侧 `BleState` 一一对应：命令返回值与 `ble:state` 事件载荷同构。 */
export interface BleState {
  phase: "idle" | "scanning" | "connecting" | "ready" | "error";
  address: string | null;
  name: string | null;
  battery: BatteryReading | null;
  noiseMode: NoiseMode | null;
  adaptiveAnc: boolean | null;
  ancGradient: AncGradient | null;
  eqPreset: EqPreset | null;
  gameMode: boolean | null;
  dualDevice: boolean | null;
  error: string | null;
  /** 已记住的耳机（配置项 `bleDevices` 的镜像；不含各条的本地记忆，见 `BleDeviceSnapshotEntry`）。 */
  devices: BleDeviceSnapshotEntry[];
  /** 自动连接的目标地址（配置项 `bleActiveAddress`）。 */
  activeAddress: string | null;
  /** 是否开着「开仓自动连接」（配置项 `bleAutoConnect`）。 */
  autoConnect: boolean;
  /** 监听器正在周期性探测耳机是否出现（此时「开仓自动连接」才真正生效）。 */
  watching: boolean;
}

/** 剪贴板记录的类型（与 Rust 侧 `ClipKind` 一一对应）。 */
export type ClipKind = "text" | "image";

/** 剪贴板列表的类型筛选（`all` 表示不筛）。 */
export type ClipKindFilter = "all" | "text" | "image";

/**
 * 与 Rust 侧 `ClipState` 一一对应：命令返回值与 `clipboard:state` 事件载荷同构。
 *
 * 事件只带状态与 `revision`，**不带条目**——列表由前端按 `revision` 重新查询，
 * 避免图片 / 长文本把广播撑大。
 */
export interface ClipState {
  enabled: boolean;
  count: number;
  /** 其中收藏的条数（收藏的记录不会被裁剪 / 清空）。 */
  pinnedCount: number;
  /** 每次历史变化自增；前端据此重查列表。 */
  revision: number;
  /** 保留天数；`0` 表示永久保留。 */
  retentionDays: number;
  captureImages: boolean;
  pasteOnClick: boolean;
  /** 已占用的存储（字节）。 */
  storageBytes: number;
}

/** 与 Rust 侧 `ClipEntryMeta` 一一对应：**不含正文**（正文按需单独取）。 */
export interface ClipEntryMeta {
  id: number;
  kind: ClipKind;
  /** 列表展示用预览（文本截断 / 图片写尺寸）。 */
  preview: string;
  /** 文本长度；图片为 0。 */
  textLength: number;
  imageWidth: number | null;
  imageHeight: number | null;
  size: number;
  createdAt: number;
  /** 是否收藏（收藏的记录置顶显示，且不会被裁剪 / 清空）。 */
  pinned: boolean;
}

/** 与 Rust 侧 `PasteOutcome` 一一对应。 */
export interface PasteOutcome {
  /** 是否真的把 Ctrl+V 发了出去。 */
  pasted: boolean;
  /** 没粘上时的原因；`null` 表示「本来就只复制」（配置关掉了自动粘贴）。 */
  reason: string | null;
}

/** 自动更新状态（与 Rust 侧 `UpdateStatus` 一一对应）。 */
export type UpdateStatus =
  | "idle"
  | "checking"
  | "upToDate"
  | "available"
  | "downloading"
  | "paused"
  | "downloaded"
  | "error";

/**
 * 与 Rust 侧 `UpdateSnapshot` 一一对应：命令返回值与 `update:state` 事件载荷同构。
 *
 * 更新源是构建期常量（Gitee 锚点 → 第三方服务器 → GitHub 兜底），不读配置；
 * 开发模式默认不检查、也不安装，联调用 `PRISM3_UPDATE_ANCHOR`（见 `src-tauri/src/updater.rs`）。
 */
export interface UpdateState {
  status: UpdateStatus;
  /** 当前应用版本（`tauri.conf.json` 的 version）。 */
  currentVersion: string;
  /** 目标版本（发现新版本之后才有值）。 */
  version: string | null;
  /** 发布说明（清单里的 `notes`）。 */
  notes: string | null;
  /** 下载进度 0-100；总长度未知时为 null。 */
  progress: number | null;
  downloadedBytes: number;
  totalBytes: number | null;
  /** 失败原因（`error` 状态）。 */
  error: string | null;
  /** 补充说明（开发模式提示 / 已取消 / 已是最新）。 */
  message: string | null;
  /** 已下载安装包的本地路径（供「打开安装包」兜底）。 */
  installerPath: string | null;
  /** 安装包下载链接（下载失败时供「下载安装包」兜底）。 */
  downloadUrl: string | null;
}

export const ipc = {
  getConfig: () => invoke<AppConfig>("get_config"),
  /** 读取壁纸并编码为 data URL（Rust 侧读文件、按文件头判类型）。 */
  readWallpaper: (path: string) => invoke<string>("read_wallpaper", { path }),
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
  /** 读取当前运行状态（窗口挂载后兜底拉取；档位变化由 `app:status` 事件广播）。 */
  getAppStatus: () => invoke<AppStatus>("get_app_status"),
  /** 采样一次系统负载并返回（可能已更新的）运行状态；由灰条窗口按可见性驱动。 */
  refreshAppStatus: () => invoke<AppStatus>("refresh_app_status"),
  /** 每次轮询变化的动态量（CPU/内存/网络/磁盘）。 */
  getSystemSnapshot: () => invoke<DynamicSnapshot>("get_system_snapshot"),
  /** 不随时间变化的静态信息（硬件型号/OS/主机名），进程启动时采集一次。 */
  getSystemStatics: () => invoke<SystemStatics>("get_system_statics"),
  /** 查询文件名索引；空关键词返回最近修改的文件。 */
  searchFiles: (query: string, limit?: number) =>
    invoke<SearchHit[]>("search_files", { query, limit }),
  /** 读取索引状态（窗口挂载后兜底拉取；变化由 `search:index-state` 事件广播）。 */
  getFileIndexState: () => invoke<FileIndexState>("get_file_index_state"),
  /** 重建索引（立即返回，进度由事件广播）。 */
  rebuildFileIndex: () => invoke<void>("rebuild_file_index"),
  /** 取消在途扫描（已建好的索引保留可用）。 */
  cancelFileIndex: () => invoke<void>("cancel_file_index"),
  /** 打开 / 在资源管理器中定位一条搜索结果（由 Rust 侧 opener 执行）。 */
  openSearchResult: (path: string, action: OpenAction) =>
    invoke<void>("open_search_result", { path, action }),
  /** 打开一个本地项目的目录（资源管理器）。 */
  openProjectFolder: (path: string) => invoke<void>("open_project_folder", { path }),
  /** 在新 cmd 窗口里执行一条项目启动命令（工作目录 = 项目目录）。 */
  launchProjectCommand: (path: string, command: string) =>
    invoke<void>("launch_project_command", { path, command }),
  /** 让快捷小窗面板进入「输入聚焦」模式；返回是否真的聚焦成功。 */
  focusQuickPanelBody: () => invoke<boolean>("focus_quick_panel_body"),
  /** 退出「输入聚焦」模式并把焦点还给用户原来的应用。 */
  blurQuickPanelBody: () => invoke<void>("blur_quick_panel_body"),
  /** 读取耳机状态（窗口挂载后兜底拉取；变化由 `ble:state` 事件广播）。 */
  getBleState: () => invoke<BleState>("get_ble_state"),
  /** 扫描一次附近的 BLE 设备（无过滤扫描 + 按地址识别：耳机广播既无名字也无服务 UUID）。 */
  scanBleDevices: () => invoke<BleCandidate[]>("scan_ble_devices"),
  /** 连接耳机：设备同一时刻只接受一个 LE 中心，厂商手机 App 占线时失败属正常结果。 */
  connectEarbuds: () => invoke<void>("connect_earbuds"),
  /** 断开耳机连接。 */
  disconnectEarbuds: () => invoke<void>("disconnect_earbuds"),
  /** 重新读取一轮设备状态（电量与各开关）。 */
  refreshEarbudState: () => invoke<void>("refresh_earbud_state"),
  /** 只读取一次电量（比整轮刷新更轻）。 */
  readEarbudBattery: () => invoke<BatteryReading>("read_earbud_battery"),
  /** 切换降噪模式。 */
  setNoiseMode: (mode: NoiseMode) => invoke<void>("set_noise_mode", { mode }),
  /** 设置降噪梯度；设备侧会顺带重新选中降噪，前端只按推送的 `ble:state` 重绘。 */
  setAncGradient: (level: AncGradient) => invoke<void>("set_anc_gradient", { level }),
  /** 开关自适应降噪（降噪段下的子项）。 */
  setAdaptiveAnc: (enabled: boolean) => invoke<void>("set_adaptive_anc", { enabled }),
  /** 切换音效预设。 */
  setEqPreset: (preset: EqPreset) => invoke<void>("set_eq_preset", { preset }),
  /** 开关游戏模式（低延迟）。 */
  setGameMode: (enabled: boolean) => invoke<void>("set_game_mode", { enabled }),
  /** 开关双设备连接。 */
  setDualDevice: (enabled: boolean) => invoke<void>("set_dual_device", { enabled }),
  /** 开关「开仓取出耳机时自动连上」；后端权威读改写，不走整份 `set_config`。 */
  setBleAutoConnect: (enabled: boolean) => invoke<void>("set_ble_auto_connect", { enabled }),
  /** 把某台设备设为「当前设备」（自动连接的目标）。 */
  setBleActiveDevice: (address: string) => invoke<void>("set_ble_active_device", { address }),
  /** 把一台设备加进已记住的列表（后端按地址去重；`name` 允许为 null）。 */
  addBleDevice: (address: string, name: string | null) =>
    invoke<void>("add_ble_device", { address, name }),
  /** 从已记住的列表里移除一台设备。 */
  removeBleDevice: (address: string) => invoke<void>("remove_ble_device", { address }),
  /** 读取剪贴板历史状态（窗口挂载后兜底拉取；变化由 `clipboard:state` 事件广播）。 */
  getClipboardState: () => invoke<ClipState>("get_clipboard_state"),
  /** 查询剪贴板历史；空关键词返回「最近入库」的条目。 */
  searchClipboard: (query: string, kind: ClipKindFilter, limit?: number) =>
    invoke<ClipEntryMeta[]>("search_clipboard", { query, kind, limit }),
  /** 取一条文本记录的完整正文（列表只拿预览）。 */
  getClipboardText: (id: number) => invoke<string>("get_clipboard_text", { id }),
  /** 取一条图片记录（默认缩略图），返回 data URL。 */
  getClipboardImage: (id: number, thumb = true) =>
    invoke<string>("get_clipboard_image", { id, thumb }),
  /** 把一条记录复制到系统剪贴板（不粘贴）。 */
  copyClipboardEntry: (id: number) => invoke<void>("copy_clipboard_entry", { id }),
  /** 写剪贴板并把焦点还给上一个窗口后粘贴（是否自动粘贴由配置决定）。 */
  pasteClipboardEntry: (id: number) => invoke<PasteOutcome>("paste_clipboard_entry", { id }),
  /** 修改一条文本记录的正文。 */
  updateClipboardText: (id: number, text: string) =>
    invoke<void>("update_clipboard_text", { id, text }),
  /** 删除一条记录（图片条目连文件一起删）。 */
  deleteClipboardEntry: (id: number) => invoke<void>("delete_clipboard_entry", { id }),
  /** 设置 / 取消一条记录的收藏。 */
  setClipboardPinned: (id: number, pinned: boolean) =>
    invoke<void>("set_clipboard_pinned", { id, pinned }),
  /** 清空全部剪贴板历史（收藏的记录会保留）。 */
  clearClipboardHistory: () => invoke<void>("clear_clipboard_history"),
  /** 立即按保留策略清理过期条目。 */
  pruneClipboardHistory: () => invoke<void>("prune_clipboard_history"),

  // ---- 速记（笔记 / 文件夹 / 标签 / 图片） ----
  /** 拉全量笔记（含正文；`trashed = true` 为回收站）。 */
  listNotes: (trashed = false) => invoke<Note[]>("list_notes", { trashed }),
  /** 取单条笔记；不存在返回 null。 */
  getNote: (noteId: number) => invoke<Note | null>("get_note", { noteId }),
  /** 新建笔记（可一次带正文 / 文件夹 / 来源）。 */
  createNoteIn: (title: string, content = "", folderId: number | null = null, sourceUrl = "") =>
    invoke<Note>("create_note_in", { title, content, folderId, sourceUrl }),
  /** 保存笔记（后端会做改名断链修复）。 */
  updateNote: (id: number, title: string, content: string) =>
    invoke<Note>("update_note", { id, title, content }),
  /** 设置 / 清除笔记的树图标（emoji）。 */
  setNoteIcon: (id: number, icon: string | null) =>
    invoke<void>("set_note_icon", { id, icon }),
  /** 移动笔记到文件夹（`folderId` 传 null 回树根）。 */
  setNoteFolder: (noteId: number, folderId: number | null) =>
    invoke<void>("set_note_folder", { noteId, folderId }),
  /** 移入回收站（软删）。 */
  trashNote: (id: number) => invoke<void>("trash_note", { id }),
  /** 从回收站还原。 */
  restoreNote: (id: number) => invoke<void>("restore_note", { id }),
  /** 永久删除（回收站内）。 */
  purgeNote: (id: number) => invoke<void>("purge_note", { id }),
  /** 清空回收站，返回清掉的条数。 */
  purgeAllTrashedNotes: () => invoke<number>("purge_all_trashed_notes"),
  /** 按配置的保留天数清理回收站。 */
  purgeExpiredNotes: () => invoke<PurgeReport>("purge_expired_notes"),
  /** 关键词搜索笔记（排除回收站）。 */
  searchNotes: (keyword: string, limit?: number) =>
    invoke<Note[]>("search_notes", { keyword, limit }),
  /** 反链面板数据。 */
  getNoteLinks: (noteId: number) => invoke<NoteLinks>("get_note_links", { noteId }),
  /** 全库双链索引重建。 */
  rebuildNoteLinks: () => invoke<void>("rebuild_note_links"),
  listNoteFolders: () => invoke<NoteFolder[]>("list_note_folders"),
  createNoteFolder: (name: string, parentId: number | null) =>
    invoke<NoteFolder>("create_note_folder", { name, parentId }),
  renameNoteFolder: (id: number, name: string) =>
    invoke<void>("rename_note_folder", { id, name }),
  deleteNoteFolder: (id: number) => invoke<void>("delete_note_folder", { id }),
  /** 拖拽落点批量写回（后端做环检测）。 */
  reorderNoteFolders: (
    moves: { id: number; parentId: number | null; sortOrder: number }[],
  ) => invoke<void>("reorder_note_folders", { moves }),
  listTags: () => invoke<Tag[]>("list_tags"),
  createTag: (name: string) => invoke<Tag>("create_tag", { name }),
  renameTag: (id: number, name: string) => invoke<void>("rename_tag", { id, name }),
  deleteTag: (id: number) => invoke<void>("delete_tag", { id }),
  getNoteTags: (noteId: number) => invoke<Tag[]>("get_note_tags", { noteId }),
  setNoteTags: (noteId: number, tagIds: number[]) =>
    invoke<void>("set_note_tags", { noteId, tagIds }),
  listNoteTags: () => invoke<NoteTagRow[]>("list_note_tags"),
  /** 导入图片（base64），返回正文应插入的 URL。 */
  importNoteImage: (dataB64: string, ext: string) =>
    invoke<string>("import_note_image", { dataB64, ext }),
  /** 孤儿图片扫描 / 清理（`dryRun = true` 只报告）。 */
  gcOrphanNoteImages: (dryRun: boolean) =>
    invoke<NoteImageGcReport>("gc_orphan_note_images", { dryRun }),
  /** 导出全部速记到指定目录（.md + front-matter + assets/）。 */
  exportNotes: (dir: string) => invoke<ExportReport>("export_notes", { dir }),
  /** 导入本应用导出的目录；`opts.overwrite` 控制 sourceUrl 命中时是否覆盖。 */
  importNotes: (dir: string, opts?: { overwrite: boolean }) =>
    invoke<ImportReport>("import_notes", { dir, opts }),
  /** 取消正在进行的导入。 */
  importNotesCancel: () => invoke<void>("import_notes_cancel"),

  /** 读取自动更新状态（窗口挂载后兜底拉取；变化由 `update:state` 事件广播）。 */
  getUpdateState: () => invoke<UpdateState>("get_update_state"),
  /** 检查更新：立即返回当前快照，结果与进度经 `update:state` 广播。 */
  checkUpdate: () => invoke<UpdateState>("check_update"),
  /** 重新安装最新版本：跳过版本比较、强制重新下载。 */
  reinstallLatest: () => invoke<UpdateState>("reinstall_latest"),
  /** 暂停下载（保留已下载字节，可继续）。 */
  pauseUpdate: () => invoke<UpdateState>("pause_update"),
  /** 恢复暂停的下载（Range 续传）。 */
  resumeUpdate: () => invoke<UpdateState>("resume_update"),
  /** 取消检查 / 下载。 */
  cancelUpdate: () => invoke<UpdateState>("cancel_update"),
  /** 安装已下载的更新并退出应用（安装器静默覆盖安装后自动拉起新版）。 */
  installUpdate: () => invoke<void>("install_update"),
  /** 在资源管理器中定位已下载的安装包。 */
  openUpdateInstaller: () => invoke<void>("open_update_installer"),
  /** 用系统浏览器打开安装包下载地址。 */
  openUpdateDownloadUrl: () => invoke<void>("open_update_download_url"),
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
