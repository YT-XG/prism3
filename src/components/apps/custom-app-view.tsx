import { ExternalLink, Globe, RotateCw } from "lucide-react";
import { useEffect, useRef, useState } from "react";

import { Button } from "@/components/ui/button";
import { ipc } from "@/lib/ipc";
import type { CustomApp } from "@/lib/apps";
import { useAppStore } from "@/stores/app-store";

/**
 * 自建应用的窗口内容。
 *
 * - `html`：用**沙箱 iframe** 渲染（`srcDoc`），自己写的脚本能跑，但拿不到父页面、
 *   也碰不到 Tauri IPC；样式不会漏到外壳上。
 * - `link`：默认**在窗口内嵌打开**（一个 iframe + 一条工具栏），不跳浏览器；
 *   设置里把「用系统浏览器打开」打开后，改回交给系统浏览器。
 *
 * 为什么还留着系统浏览器这条路：不少站点在响应头里写了 `X-Frame-Options: DENY`
 * 或 CSP `frame-ancestors`，WebView 里只会得到一片空白——那种站只能让浏览器打开。
 */
export function CustomAppView({ app }: { app: CustomApp }) {
  const openExternal = useAppStore((state) => state.config?.customAppOpenExternal ?? false);

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

  return openExternal ? <ExternalLinkView app={app} /> : <EmbeddedLinkView app={app} />;
}

/** 内嵌模式：工具栏 + iframe。刷新用 `key` 强制重建，避免污染浏览历史。 */
function EmbeddedLinkView({ app }: { app: CustomApp }) {
  const [nonce, setNonce] = useState(0);
  const [loaded, setLoaded] = useState(false);
  const [stalled, setStalled] = useState(false);

  // 有些站点禁止被内嵌、或干脆连不上：load 可能一直不来，别让人对着「正在加载…」干等。
  useEffect(() => {
    if (loaded) return;
    const timer = window.setTimeout(() => setStalled(true), 8000);
    return () => window.clearTimeout(timer);
  }, [loaded, nonce]);

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div
        className="flex h-9 shrink-0 items-center gap-1.5 border-b px-2.5"
        style={{ borderColor: "var(--window-border)" }}
      >
        <Globe className="size-3.5 shrink-0 text-muted-foreground" aria-hidden />
        <span className="min-w-0 flex-1 truncate text-[11px] text-muted-foreground" title={app.url}>
          {app.url}
        </span>
        <Button
          variant="ghost"
          size="icon-xs"
          aria-label="刷新"
          title="刷新"
          onClick={() => {
            setLoaded(false);
            setNonce((value) => value + 1);
          }}
        >
          <RotateCw />
        </Button>
        <Button
          variant="ghost"
          size="icon-xs"
          aria-label="在系统浏览器中打开"
          title="在系统浏览器中打开"
          onClick={() => void ipc.openExternalUrl(app.url)}
        >
          <ExternalLink />
        </Button>
      </div>

      <div className="relative min-h-0 flex-1">
        {loaded ? null : (
          <div className="absolute inset-0 z-10 flex flex-col items-center justify-center gap-2 bg-[var(--window-body)] px-6 text-center">
            {stalled ? (
              <>
                <p className="text-[12px] font-medium text-foreground">这个页面没能加载出来</p>
                <p className="max-w-[40ch] text-[11px] leading-relaxed text-muted-foreground">
                  有的站点禁止被内嵌（X-Frame-Options / CSP），也可能是网络没通。
                </p>
                <Button size="sm" variant="outline" onClick={() => void ipc.openExternalUrl(app.url)}>
                  <ExternalLink data-icon="inline-start" />
                  用系统浏览器打开
                </Button>
              </>
            ) : (
              <span className="text-[11px] text-muted-foreground">正在加载…</span>
            )}
          </div>
        )}
        <iframe
          key={nonce}
          src={app.url}
          title={app.name}
          sandbox="allow-scripts allow-same-origin allow-forms allow-popups allow-popups-to-escape-sandbox"
          referrerPolicy="no-referrer"
          onLoad={() => setLoaded(true)}
          className="h-full w-full border-0 bg-white"
        />
      </div>
    </div>
  );
}

/** 外部模式：窗口里只留一张「说明书」卡片，打开即交给系统浏览器。 */
function ExternalLinkView({ app }: { app: CustomApp }) {
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
        当前设置是「用系统浏览器打开」；在设置里关掉就会改成应用内嵌打开。
      </p>
    </div>
  );
}
