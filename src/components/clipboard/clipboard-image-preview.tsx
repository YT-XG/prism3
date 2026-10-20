import { useEffect, useState } from "react";

import { createPortal } from "react-dom";

import { ClipboardThumb } from "@/components/clipboard/clipboard-thumb";
import { ipc } from "@/lib/ipc";

/** 浮层与视口边缘的留白。 */
const MARGIN = 12;
/** 浮层上限（含内边距），同时用于贴边时的位置钳制。 */
const MAX = 320;

/**
 * 原图缓存：**只在悬停时按需拉取**（原图 base64 可能好几 MB，不像缩略图那样可以多留），
 * 命中即秒开；超过上限按插入顺序淘汰最旧的几张。
 */
const FULL_CACHE_LIMIT = 3;
const fullCache = new Map<number, string>();
const inflight = new Map<number, Promise<string>>();

function loadFull(id: number): Promise<string> {
  const cached = fullCache.get(id);
  if (cached) return Promise.resolve(cached);

  const pending = inflight.get(id);
  if (pending) return pending;

  const request = ipc
    .getClipboardImage(id, false)
    .then((url) => {
      fullCache.set(id, url);
      if (fullCache.size > FULL_CACHE_LIMIT) {
        const oldest = fullCache.keys().next().value;
        if (oldest !== undefined) fullCache.delete(oldest);
      }
      return url;
    })
    .finally(() => inflight.delete(id));

  inflight.set(id, request);
  return request;
}

/**
 * 剪贴板图片的「悬停放大预览」。
 *
 * - 先立刻显示缓存的 256px 缩略图（不放大、不失真），原图载好后无缝替换成高清图；
 * - **必须用 portal 挂到 `body`**：首页卡片是 `overflow-hidden`，放在卡片里的绝对定位浮层
 *   会被裁掉；
 * - 浮层 `pointer-events-none`，不会把鼠标从被悬停的行上抢走（否则会反复闪烁）；
 * - 图片仍经命令回传 data URL（**不给前端开文件路径权限**）。
 */
export function ClipboardImagePreview({ id, anchor }: { id: number; anchor: DOMRect }) {
  const [full, setFull] = useState<string | null>(() => fullCache.get(id) ?? null);

  useEffect(() => {
    setFull(fullCache.get(id) ?? null);
    let active = true;
    void loadFull(id)
      .then((url) => {
        if (active) setFull(url);
      })
      .catch(() => {
        // 原图取不到（文件被外部删掉 / 记录刚清掉）：保持缩略图即可，不抛错。
      });
    return () => {
      active = false;
    };
  }, [id]);

  // 贴在锚点右侧并垂直居中于该行；越界则钳回视口内。
  const left = Math.max(
    MARGIN,
    Math.min(anchor.right + 12, window.innerWidth - MAX - MARGIN),
  );
  const top = Math.max(
    MARGIN,
    Math.min(anchor.top + anchor.height / 2 - MAX / 2, window.innerHeight - MAX - MARGIN),
  );

  return createPortal(
    <div
      className="pointer-events-none fixed z-50 rounded-lg border border-border bg-popover p-1.5 shadow-xl"
      style={{ left, top, maxWidth: MAX, maxHeight: MAX }}
    >
      {full ? (
        <img
          src={full}
          alt=""
          draggable={false}
          className="max-h-[300px] max-w-[300px] rounded object-contain"
        />
      ) : (
        // 缩略图按原尺寸显示（256 长边以内不放大 ⇒ 不糊）；尚未缓存时给个体面的占位块。
        <ClipboardThumb
          id={id}
          className="max-h-[300px] min-h-24 min-w-24 max-w-[300px]"
        />
      )}
    </div>,
    document.body,
  );
}
