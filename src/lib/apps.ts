import {
  Activity,
  ClipboardList,
  FolderGit2,
  House,
  ListChecks,
  MonitorSmartphone,
  NotebookPen,
  Search,
  Settings,
  type LucideIcon,
} from "lucide-react";

import type { AppView } from "@/stores/app-store";

/** 一个「应用」在外壳里的身份：图标、标题、默认窗口尺寸、图标底座配色。 */
export interface AppDef {
  id: AppView;
  /** 图标下的名字（桌面 / 任务栏 / 开始菜单共用）。 */
  label: string;
  /** 窗口标题栏与开始菜单里的说明。 */
  title: string;
  description: string;
  icon: LucideIcon;
  /** 图标底座的两段渐变（参考站每个应用一个色相）。 */
  tint: [string, string];
  /** 默认窗口尺寸（px）。 */
  size: { w: number; h: number };
  /** 允许缩到的最小尺寸（px）。 */
  min: { w: number; h: number };
}

/**
 * 桌面上的九个应用。`home` 是原来的 Bento 工作台，
 * 现在作为「工作台」窗口打开——桌面本身只放小组件，把首页还给壁纸。
 */
export const APPS: AppDef[] = [
  {
    id: "home",
    label: "首页",
    title: "首页",
    description: "可拖拽的模块网格",
    icon: House,
    tint: ["#5b9bff", "#187aff"],
    size: { w: 1120, h: 720 },
    min: { w: 720, h: 460 },
  },
  {
    id: "search",
    label: "搜索",
    title: "搜索",
    description: "按文件名找本机文件",
    icon: Search,
    tint: ["#4ade9f", "#0ea5a5"],
    size: { w: 880, h: 620 },
    min: { w: 560, h: 420 },
  },
  {
    id: "clipboard",
    label: "剪贴板",
    title: "剪贴板",
    description: "文本与图片历史",
    icon: ClipboardList,
    tint: ["#ffc061", "#f08a1d"],
    size: { w: 900, h: 660 },
    min: { w: 620, h: 460 },
  },
  {
    id: "notes",
    label: "速记",
    title: "速记",
    description: "文件夹、标签与双链",
    icon: NotebookPen,
    tint: ["#5ee0a6", "#16a06b"],
    size: { w: 1140, h: 740 },
    min: { w: 760, h: 500 },
  },
  {
    id: "projects",
    label: "项目",
    title: "项目",
    description: "一键启动本地项目",
    icon: FolderGit2,
    tint: ["#a78bfa", "#6b5bf0"],
    size: { w: 980, h: 680 },
    min: { w: 640, h: 460 },
  },
  {
    id: "work",
    label: "工作",
    title: "工作记录",
    description: "完成台账与周报",
    icon: ListChecks,
    tint: ["#fb8fb0", "#e04a72"],
    size: { w: 1100, h: 720 },
    min: { w: 720, h: 480 },
  },
  {
    id: "monitor",
    label: "监控",
    title: "系统监控",
    description: "CPU / 内存 / 显卡",
    icon: Activity,
    tint: ["#7c8cff", "#4553dc"],
    size: { w: 1020, h: 700 },
    min: { w: 640, h: 460 },
  },
  {
    id: "devices",
    label: "设备",
    title: "设备",
    description: "耳机与无线外设",
    icon: MonitorSmartphone,
    tint: ["#5ac8fa", "#0a84c9"],
    size: { w: 920, h: 660 },
    min: { w: 600, h: 440 },
  },
  {
    id: "settings",
    label: "设置",
    title: "设置",
    description: "外观、窗口与数据",
    icon: Settings,
    tint: ["#94a3b8", "#5b6675"],
    size: { w: 960, h: 700 },
    min: { w: 640, h: 480 },
  },
];

export const APP_MAP = new Map<AppView, AppDef>(APPS.map((app) => [app.id, app]));

export function appDef(id: AppView): AppDef {
  const def = APP_MAP.get(id);
  if (!def) throw new Error(`未知应用：${id}`);
  return def;
}

/** 桌面图标坞的顺序：常用 → 系统。 */
export const DOCK_ORDER: AppView[] = [
  "home",
  "search",
  "clipboard",
  "notes",
  "projects",
  "work",
  "monitor",
  "devices",
  "settings",
];
