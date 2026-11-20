import { Plus, Search } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";

import { useAppVersion } from "@/hooks/use-app-version";
import { useAppCatalog, type AppMeta } from "@/hooks/use-app-catalog";
import { cn } from "@/lib/utils";
import { useAppStore, type AppView } from "@/stores/app-store";
import { useWindowStore } from "@/stores/window-store";

/**
 * 开始菜单（520px，位于任务栏上方左侧）。
 *
 * 结构照参考站：一个搜索框 + 应用宫格 + 底部一条说明。
 * 玻璃用「窗口级」的不透明度（它盖住壁纸，必须挡住后面的图标）。
 */
export function StartMenu({ open, onClose }: { open: boolean; onClose: () => void }) {
  const [query, setQuery] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);
  const version = useAppVersion();
  const setActiveView = useAppStore((state) => state.setActiveView);
  const openCustomAppDialog = useAppStore((state) => state.openCustomAppDialog);
  const openWindow = useWindowStore((state) => state.open);
  const catalog = useAppCatalog();

  const apps = useMemo(() => {
    const lower = query.trim().toLowerCase();
    if (!lower) return catalog;
    return catalog.filter((app) =>
      `${app.label}${app.title}${app.description}`.toLowerCase().includes(lower),
    );
  }, [catalog, query]);

  useEffect(() => {
    inputRef.current?.focus();
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  const launch = (app: AppMeta) => {
    if (app.kind === "builtin") {
      setActiveView(app.id as AppView);
    } else {
      openWindow(app.id, app.size);
    }
    onClose();
  };

  return (
    <>
      <div className="fixed inset-0 z-40" aria-hidden onPointerDown={onClose} />
      <div
        role="menu"
        aria-label="开始菜单"
        className={cn(
          "absolute left-2 z-50 flex w-[520px] flex-col overflow-hidden rounded-2xl border",
          open ? "menu-in" : "menu-out",
        )}
        style={{
          bottom: "calc(var(--taskbar-h) + 10px)",
          background: "var(--window-bg)",
          borderColor: "var(--window-border)",
          boxShadow: "0 30px 70px -32px rgba(16, 40, 80, 0.55)",
          backdropFilter: "blur(28px) saturate(1.3)",
          WebkitBackdropFilter: "blur(28px) saturate(1.3)",
        }}
      >
        <div className="flex items-center gap-2 border-b px-3 py-2.5" style={{ borderColor: "var(--taskbar-border)" }}>
          <Search className="size-4 shrink-0 text-muted-foreground" />
          <input
            ref={inputRef}
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="搜索应用…"
            aria-label="搜索应用"
            className="h-7 w-full bg-transparent text-[13px] text-foreground placeholder:text-muted-foreground focus:outline-none"
          />
        </div>

        <div className="grid max-h-[336px] grid-cols-4 gap-1 overflow-y-auto p-2">
          {apps.map((app) => {
            const Icon = app.icon;
            return (
              <button
                key={app.id}
                type="button"
                role="menuitem"
                onClick={() => launch(app)}
                className={cn(
                  "flex flex-col items-center gap-1.5 rounded-xl px-1 pt-2.5 pb-2 transition-colors",
                  "hover:bg-black/5 focus-visible:bg-black/5 focus-visible:outline-none dark:hover:bg-white/8",
                )}
              >
                <span
                  className="flex size-[40px] items-center justify-center rounded-[12px] text-white shadow-[0_6px_14px_-8px_rgba(16,40,80,.6)]"
                  style={{
                    backgroundImage: `linear-gradient(150deg, ${app.tint[0]}, ${app.tint[1]})`,
                  }}
                >
                  {Icon ? (
                    <Icon className="size-[19px]" strokeWidth={1.9} />
                  ) : (
                    <span className="text-[18px] leading-none">{app.emoji ?? "📦"}</span>
                  )}
                </span>
                <span className="text-[11px] leading-none font-medium text-foreground">
                  {app.label}
                </span>
              </button>
            );
          })}

          {apps.length === 0 ? (
            <p className="col-span-4 px-2 py-6 text-center text-[12px] text-muted-foreground">
              没有匹配的应用
            </p>
          ) : null}
        </div>

        <div
          className="flex items-center justify-between border-t px-3 py-2 text-[10.5px] text-muted-foreground"
          style={{ borderColor: "var(--taskbar-border)" }}
        >
          <button
            type="button"
            onClick={() => {
              openCustomAppDialog();
              onClose();
            }}
            className="flex items-center gap-1 rounded-md px-1.5 py-0.5 text-[10.5px] font-medium transition-colors hover:bg-black/5 hover:text-foreground dark:hover:bg-white/8"
          >
            <Plus className="size-3" aria-hidden />
            创建应用
          </button>
          <span>
            PRISM3{version ? ` v${version}` : ""} · Esc 关闭
          </span>
        </div>
      </div>
    </>
  );
}
