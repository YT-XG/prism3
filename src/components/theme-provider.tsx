import { useEffect, type ReactNode } from "react";

import { useAppStore } from "@/stores/app-store";

/**
 * 外观的三根轴，与 Rust 侧 `AppConfig` 一一对应：
 * - 模式 `theme`：`system | light | dark` → `<html class="dark">`
 * - 配色 `themePreset`：只改页面渐变与玻璃染色的色相（见 index.css 的 `[data-preset]`）
 * - 强调色 `accentColor`：`#rrggbb`，内联写成 `--brand`，主按钮/焦点环/选中态全部派生
 */
export interface ThemePreset {
  id: string;
  label: string;
  /** 三个色相 + 饱和度，与 index.css 的预设块保持一致，供设置页画预览色板 */
  hues: [number, number, number];
  sat: number;
}

export const THEME_PRESETS: ThemePreset[] = [
  { id: "aurora", label: "极光", hues: [245, 330, 217], sat: 82 },
  { id: "slate", label: "石墨", hues: [220, 210, 232], sat: 18 },
  { id: "ocean", label: "深海", hues: [205, 190, 228], sat: 70 },
  { id: "sage", label: "青苔", hues: [150, 170, 105], sat: 48 },
  { id: "forest", label: "森林", hues: [135, 160, 95], sat: 58 },
  { id: "sunset", label: "落日", hues: [22, 340, 45], sat: 80 },
  { id: "rose", label: "玫瑰", hues: [335, 350, 300], sat: 78 },
  { id: "violet", label: "紫罗兰", hues: [265, 290, 240], sat: 76 },
  { id: "amber", label: "琥珀", hues: [38, 24, 55], sat: 78 },
  { id: "candy", label: "糖果", hues: [318, 200, 280], sat: 86 },
];

export const DEFAULT_PRESET = THEME_PRESETS[0].id;

/** 8 个预设强调色（与 index.css 默认值同源；选中后内联覆盖 --brand）。 */
export const ACCENT_COLORS: { value: string; label: string }[] = [
  { value: "#5b5bf5", label: "靛蓝" },
  { value: "#8b5cf6", label: "紫罗兰" },
  { value: "#3b82f6", label: "湖蓝" },
  { value: "#06b6d4", label: "青" },
  { value: "#10b981", label: "翡翠" },
  { value: "#f59e0b", label: "琥珀" },
  { value: "#f97316", label: "橙" },
  { value: "#f43f5e", label: "玫瑰" },
];

export const DEFAULT_ACCENT = ACCENT_COLORS[0].value;

const HEX_COLOR = /^#[0-9a-f]{6}$/i;

/** 非法/缺失的强调色回退到默认值；配置文件可能被手改。 */
export function resolveAccent(value?: string | null): string {
  return value && HEX_COLOR.test(value) ? value.toLowerCase() : DEFAULT_ACCENT;
}

/** 未知预设 id 回退到默认预设（CSS 里没有对应块时用 :root 的极光）。 */
export function resolvePreset(value?: string | null): string {
  return THEME_PRESETS.some((preset) => preset.id === value) ? (value as string) : DEFAULT_PRESET;
}

/** 设置页的色板预览：三个色相拉一条斜向渐变，与页面背景同构。 */
export function presetGradient(preset: ThemePreset): string {
  const [a, b, c] = preset.hues;
  const s = preset.sat;
  return `linear-gradient(140deg, hsl(${a} ${s}% 62%) 0%, hsl(${b} ${s}% 68%) 52%, hsl(${c} ${s}% 60%) 100%)`;
}

const STORAGE = {
  theme: "prism3:theme",
  preset: "prism3:preset",
  accent: "prism3:accent",
} as const;

/**
 * 主题读取接口，签名与 next-themes 的 useTheme 兼容，
 * 使 shadcn 生成的主题相关组件（如 sonner）可直接复用。
 */
export function useTheme() {
  return { theme: useAppStore((state) => state.config?.theme ?? "system") };
}

/**
 * 把配置里的外观偏好落到 <html> 上：
 * `.dark`（模式）、`data-preset`（配色）、内联 `--brand`（强调色）。
 *
 * 同时把三项原样写进 localStorage —— 启动时配置还没异步读回来，
 * index.html 的内联脚本靠它先画出正确底色，避免暗色/换色时闪一帧。
 */
export function ThemeProvider({ children }: { children: ReactNode }) {
  const theme = useAppStore((state) => state.config?.theme ?? "system");
  const themePreset = useAppStore((state) => state.config?.themePreset);
  const accentColor = useAppStore((state) => state.config?.accentColor);

  useEffect(() => {
    const root = document.documentElement;
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const preset = resolvePreset(themePreset);
    const accent = resolveAccent(accentColor);

    const apply = () => {
      const dark = theme === "dark" || (theme === "system" && media.matches);
      root.classList.toggle("dark", dark);
      root.style.colorScheme = dark ? "dark" : "light";
      root.dataset.theme = dark ? "dark" : "light";
      root.dataset.preset = preset;
      root.style.setProperty("--brand", accent);

      try {
        window.localStorage.setItem(STORAGE.theme, theme);
        window.localStorage.setItem(STORAGE.preset, preset);
        window.localStorage.setItem(STORAGE.accent, accent);
      } catch {
        // 存储不可用（隐私模式等）时按系统主题走，不影响应用。
      }
    };

    apply();
    if (theme !== "system") return;

    media.addEventListener("change", apply);
    return () => media.removeEventListener("change", apply);
  }, [theme, themePreset, accentColor]);

  return <>{children}</>;
}
