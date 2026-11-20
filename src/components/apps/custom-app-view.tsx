import { ExternalLink, Globe } from "lucide-react";
import { useEffect, useRef } from "react";

import { Button } from "@/components/ui/button";
import { ipc } from "@/lib/ipc";
import type { CustomApp } from "@/lib/apps";

/**
 * 自建应用的窗口内容。
 *
 * - `link`：不在应用内嵌网页（WebView 里没有多标签、也绕不开 X-Frame-Options），
 *   打开即交给系统浏览器，窗口本身留一张「说明书」卡片，随时可以再点一次。
 * - `html`：用**沙箱 iframe** 渲染（`allow-scripts` 但不给 `allow-same-origin`），
 *   自己写的脚本能跑，但拿不到父页面、也碰不到 Tauri IPC；样式不会漏到外壳上。
 */
export function CustomAppView({ app }: { app: CustomApp }) {
  if (app.kind === "html") {
    return (
      <iframe
        title={app.name}
        srcDoc={app.html}
        sandbox="allow-scripts allow-forms allow-modals allow-popups"
        className="h-full w-full border-0 bg-white"
      />
    );
  }

  return <LinkAppView app={app} />;
}

function LinkAppView({ app }: { app: CustomApp }) {
  const openedOnce = useRef(false);

  // 点图标就是要看这个站：第一次挂载直接交给浏览器，窗口留着当「再来一次」的入口。
  useEffect(() => {
    if (openedOnce.current) return;
    openedOnce.current = true;
    void ipc.openExternalUrl(app.url);
  }, [app.url]);

  return (
    <div className="flex h-full flex-col items-center justify-center gap-4 px-8 text-center">
      <span
        className="flex size-16 items-center justify-center rounded-[18px] text-[30px] text-white"
        style={{
          backgroundImage: `linear-gradient(150deg, ${app.tint[0]}, ${app.tint[1]})`,
        }}
      >
        {app.icon || "🔗"}
      </span>

      <div className="max-w-[46ch]">
        <p className="font-heading text-[15px] font-semibold text-foreground">{app.name}</p>
        <p className="mt-1.5 truncate text-[12px] text-muted-foreground" title={app.url}>
          {app.url}
        </p>
      </div>

      <div className="flex items-center gap-2">
        <Button size="sm" onClick={() => void ipc.openExternalUrl(app.url)}>
          <ExternalLink data-icon="inline-start" />
          在浏览器中打开
        </Button>
        <Button
          size="sm"
          variant="outline"
          onClick={() => void navigator.clipboard.writeText(app.url)}
        >
          <Globe data-icon="inline-start" />
          复制链接
        </Button>
      </div>

      <p className="text-[11px] text-muted-foreground">
        链接型应用不内嵌网页：复制地址在自己习惯的浏览器里打开。
      </p>
    </div>
  );
}
