import { useEffect, useState } from "react";

import { ipc } from "@/lib/ipc";
import { cn } from "@/lib/utils";

/**
 * 缩略图缓存：同一张图在列表里反复进出（搜索、翻页、重查）时不再重复走 IPC。
 *
 * 图片经命令以 data URL 回传（**不给前端开文件路径权限**），每次都是几十 KB 的
 * base64，因此必须缓存；超过上限就按插入顺序淘汰最旧的几张。
 */
const CACHE_LIMIT = 200;
const cache = new Map<number, string>();
const inflight = new Map<number, Promise<string>>();

function remember(id: number, url: string) {
  cache.set(id, url);
  if (cache.size > CACHE_LIMIT) {
    const oldest = cache.keys().next().value;
    if (oldest !== undefined) cache.delete(oldest);
  }
}

function load(id: number): Promise<string> {
  const cached = cache.get(id);
  if (cached) return Promise.resolve(cached);

  const pending = inflight.get(id);
  if (pending) return pending;

  const request = ipc
    .getClipboardImage(id, true)
    .then((url) => {
      remember(id, url);
      return url;
    })
    .finally(() => inflight.delete(id));

  inflight.set(id, request);
  return request;
}

/**
 * 一条图片记录的缩略图（懒加载：只有真的渲染出来才去取）。
 *
 * 取不到（文件被外部删掉 / 记录刚被清理）时显示占位块，不抛错也不阻塞列表。
 */
export function ClipboardThumb({ id, className }: { id: number; className?: string }) {
  const [url, setUrl] = useState<string | null>(() => cache.get(id) ?? null);

  useEffect(() => {
    if (url) return;

    let active = true;
    void load(id)
      .then((next) => {
        if (active) setUrl(next);
      })
      .catch(() => {
        // 缩略图取不到不影响其它功能：保持占位块即可。
      });

    return () => {
      active = false;
    };
  }, [id, url]);

  if (!url) {
    return <div className={cn("shrink-0 rounded bg-muted", className)} aria-hidden />;
  }

  return (
    <img
      src={url}
      alt=""
      draggable={false}
      className={cn("shrink-0 rounded bg-muted object-cover", className)}
    />
  );
}
