import { useEffect, useState } from "react";

import { ipc } from "@/lib/ipc";
import { useAppStore } from "@/stores/app-store";

/**
 * 桌面壁纸：默认是强调色派生的 135° 斜纹（`--desktop-bg`），
 * 配置了图片时整层换成模糊照片 + 可调蒙版，并给 `<html>` 挂 `data-wallpaper`
 * 让外壳切到「壁纸态」（窗口稍微透一点，图标文字补描边）。
 */
export function DesktopWallpaper() {
  const path = useAppStore((state) => state.config?.wallpaper ?? "");
  const blur = useAppStore((state) => state.config?.wallpaperBlur ?? 18);
  const veil = useAppStore((state) => state.config?.wallpaperVeil ?? 35);
  const [src, setSrc] = useState<string | null>(null);

  // 只读一次：配置里存路径，字节由 Rust 读成 data URL（不经过 fs 插件 scope）。
  useEffect(() => {
    let active = true;
    if (!path) {
      setSrc(null);
      return;
    }
    void ipc
      .readWallpaper(path)
      .then((data) => {
        if (active) setSrc(data);
      })
      .catch(() => {
        if (active) setSrc(null);
      });
    return () => {
      active = false;
    };
  }, [path]);

  useEffect(() => {
    if (!src) return;
    const root = document.documentElement;
    root.dataset.wallpaper = "1";
    return () => {
      delete root.dataset.wallpaper;
    };
  }, [src]);

  return (
    <>
      <div className="desktop-wallpaper" aria-hidden="true" />
      {src ? (
        <div className="wallpaper-layer" aria-hidden="true">
          <div
            className="wallpaper-image"
            style={{ backgroundImage: `url("${src}")`, filter: `blur(${blur}px)` }}
          />
          <div className="wallpaper-veil" style={{ opacity: veil / 100 }} />
        </div>
      ) : null}
    </>
  );
}
