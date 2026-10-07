import { useEffect, type ReactNode } from "react";

import { useAppStore } from "@/stores/app-store";

/**
 * 主题读取接口，签名与 next-themes 的 useTheme 兼容，
 * 使 shadcn 生成的主题相关组件（如 sonner）可直接复用。
 */
export function useTheme() {
  return { theme: useAppStore((state) => state.config?.theme ?? "system") };
}

/**
 * 把配置中的主题偏好映射为 <html> 上的 dark class。
 * theme 为 "system" 时跟随系统，并在系统主题变化时同步。
 */
export function ThemeProvider({ children }: { children: ReactNode }) {
  const theme = useAppStore((state) => state.config?.theme ?? "system");

  useEffect(() => {
    const root = document.documentElement;
    const media = window.matchMedia("(prefers-color-scheme: dark)");

    const apply = () => {
      const dark = theme === "dark" || (theme === "system" && media.matches);
      root.classList.toggle("dark", dark);
    };

    apply();
    if (theme !== "system") return;

    media.addEventListener("change", apply);
    return () => media.removeEventListener("change", apply);
  }, [theme]);

  return <>{children}</>;
}
