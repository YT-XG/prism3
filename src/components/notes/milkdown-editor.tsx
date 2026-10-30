import { useCallback, useEffect, useRef, useState, type ClipboardEvent } from "react";

import "@milkdown/crepe/theme/common/style.css";
import "@milkdown/crepe/theme/frame.css";
import "@/milkdown.css";

import { Crepe } from "@milkdown/crepe";
import { editorViewCtx } from "@milkdown/kit/core";
import { NodeSelection, type EditorState } from "@milkdown/kit/prose/state";
import type { EditorView } from "@milkdown/kit/prose/view";
import { error as logError } from "@tauri-apps/plugin-log";
import { toast } from "sonner";

import { saveNoteImageFile } from "@/lib/notes/image-upload";
import { ipc, type Note } from "@/lib/ipc";

/** 笔记图片协议 URL 前缀（与 Rust `note_image::NOTE_URL_PREFIX` 保持一致）。 */
const NOTE_IMAGE_URL_PREFIX = "http://prism3-note.localhost/";
/** 合法笔记图片文件名：16 位十六进制哈希 + 白名单扩展（与 Rust 侧同一口径）。 */
const NOTE_IMAGE_NAME = /^[0-9a-f]{16}\.(png|jpe?g|webp|bmp|gif)$/i;

interface MilkdownEditorProps {
  /** 当前笔记 id：变化即销毁重建（不同笔记不能共用实例）。 */
  noteId: number;
  initialContent: string;
  /** 是否可编辑（`preview` 模式 / 分屏右侧只读）。 */
  editable: boolean;
  /** 正文变化（内部 600ms 防抖由上层负责）。 */
  onChange: (markdown: string) => void;
  /** 点击正文里的 `[标题](note/<id>)` 双链跳转。 */
  onOpenNote: (id: number) => void;
  /** `[[` 补全候选。 */
  notes: Note[];
}

interface WikiSuggest {
  /** 文档位置：`[[` 起点。 */
  from: number;
  /** 文档位置：光标（替换终点）。 */
  to: number;
  query: string;
  items: Note[];
  index: number;
  /** 相对编辑容器的像素坐标。 */
  x: number;
  y: number;
}

/** 把 ProseMirror 的可编辑性同步到给定值。 */
function syncEditable(view: EditorView, editable: boolean) {
  view.setProps({ editable: () => editable });
}

/**
 * Milkdown Crepe 编辑器（速记正文）：实时预览 / 只读预览共用一个组件，靠 `editable` 区分。
 *
 * 关键约束（踩坑记录）：
 * 1. **StrictMode 双挂载**：`create()` 是异步的，挂载期间组件可能已卸载或换笔记——
 *    用 `cancelled` 标志，`create()` 返回后若已取消就立刻 `destroy()`。
 * 2. **监听必须在 `create()` 之后挂**：先挂会在初始化时触发一次假保存。
 * 3. 变化只走 `markdownUpdated`，由上层防抖落库。
 */
export function MilkdownEditor({
  noteId,
  initialContent,
  editable,
  onChange,
  onOpenNote,
  notes,
}: MilkdownEditorProps) {
  const rootRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const [suggest, setSuggest] = useState<WikiSuggest | null>(null);

  const changeRef = useRef(onChange);
  changeRef.current = onChange;
  const openRef = useRef(onOpenNote);
  openRef.current = onOpenNote;
  const notesRef = useRef(notes);
  notesRef.current = notes;
  const suggestRef = useRef<WikiSuggest | null>(null);
  suggestRef.current = suggest;

  // 依据当前文本块生成 `[[` 补全候选；无匹配则收起。
  const refreshSuggest = useCallback(() => {
    const view = viewRef.current;
    if (!view) return;
    const { selection } = view.state;
    if (!selection.empty) {
      setSuggest(null);
      return;
    }
    const $from = selection.$from;
    // 当前文本块起点到光标的纯文本（`\0` 分隔行内节点，避免把图片等算进查询）。
    const textBefore = $from.parent.textBetween(0, $from.parentOffset, undefined, "\0");
    const match = /\[\[([^[\]\n]*)$/.exec(textBefore);
    if (!match) {
      setSuggest(null);
      return;
    }
    const query = match[1];
    const items = notesRef.current
      .filter((note) => (note.title || "无标题笔记").toLowerCase().includes(query.toLowerCase()))
      .slice(0, 8);
    if (items.length === 0) {
      setSuggest(null);
      return;
    }
    const from = $from.start() + match.index;
    let x = 0;
    let y = 0;
    const root = rootRef.current;
    if (root) {
      const caret = view.coordsAtPos(selection.from);
      const rect = root.getBoundingClientRect();
      x = caret.left - rect.left;
      y = caret.bottom - rect.top;
    }
    setSuggest({
      from,
      to: selection.from,
      query,
      items,
      index: 0,
      x,
      y,
    });
  }, []);

  const applySuggest = useCallback((note: Note) => {
    const view = viewRef.current;
    const current = suggestRef.current;
    if (!view || !current) return;
    const title = note.title || "无标题笔记";
    const text = `[${title}](note/${note.id})`;
    const tr = view.state.tr.insertText(text, current.from, current.to);
    view.dispatch(tr);
    view.focus();
    setSuggest(null);
  }, []);

  /**
   * 右键「复制」一张笔记图片：接管剪贴板，写真正的位图而不是 Markdown 文本。
   *
   * 编辑器默认把图片节点序列化成 `![alt](url)`，粘到微信 / 文档里只是一行字。这里在
   * **捕获阶段**拦住「选中内容恰好是一张笔记图片」的复制：`preventDefault` 挡掉浏览器
   * 默认复制，`stopPropagation` 挡掉 ProseMirror 自己的 copy 处理（否则它仍会把 Markdown
   * 写进剪贴板，把位图覆盖掉）。混了文字的选择不干预。
   */
  const handleCopy = (event: ClipboardEvent<HTMLDivElement>) => {
    const view = viewRef.current;
    if (!view) return;
    const src =
      selectedNoteImageSrc(view.state.selection) ?? selectedNoteImageSrcFromDom();
    if (!src) return;
    event.preventDefault();
    event.stopPropagation();
    void ipc.copyNoteImage(src).catch((cause) => {
      const message = `复制图片失败：${String(cause)}`;
      void logError(message);
      toast.error(message);
    });
  };

  useEffect(() => {
    const root = rootRef.current;
    if (!root) return;
    let cancelled = false;
    let teardown: (() => void) | undefined;

    const crepe = new Crepe({
      root,
      defaultValue: initialContent,
      features: { [Crepe.Feature.AI]: false },
      featureConfigs: {
        [Crepe.Feature.ImageBlock]: {
          onUpload: (file: File) => saveNoteImageFile(file),
          inlineOnUpload: (file: File) => saveNoteImageFile(file),
          blockOnUpload: (file: File) => saveNoteImageFile(file),
        },
      },
    });

    void crepe.create().then(() => {
      if (cancelled) {
        void crepe.destroy();
        return;
      }
      crepe.editor.action((ctx) => {
        const view = ctx.get(editorViewCtx);
        viewRef.current = view;
        syncEditable(view, editable);

        const dom = view.dom;
        const onInput = () => {
          refreshSuggest();
        };
        const onKeyDown = (event: KeyboardEvent) => {
          const current = suggestRef.current;
          if (!current) return;
          if (event.key === "ArrowDown" || event.key === "ArrowUp") {
            event.preventDefault();
            const delta = event.key === "ArrowDown" ? 1 : -1;
            const next = (current.index + delta + current.items.length) % current.items.length;
            setSuggest({ ...current, index: next });
          } else if (event.key === "Enter") {
            event.preventDefault();
            applySuggest(current.items[current.index]);
          } else if (event.key === "Escape") {
            event.preventDefault();
            setSuggest(null);
          }
        };
        const onClick = (event: MouseEvent) => {
          const target = event.target as HTMLElement | null;
          const anchor = target?.closest?.("a");
          const href = anchor?.getAttribute("href") ?? "";
          const match = /^note\/(\d+)$/.exec(href);
          if (match) {
            event.preventDefault();
            openRef.current(Number(match[1]));
          }
        };
        // IME 组合期间不刷新候选（compositionend 再补一次）。
        const onCompositionEnd = () => refreshSuggest();

        dom.addEventListener("input", onInput);
        dom.addEventListener("keydown", onKeyDown, true);
        dom.addEventListener("click", onClick);
        dom.addEventListener("compositionend", onCompositionEnd);

        teardown = () => {
          dom.removeEventListener("input", onInput);
          dom.removeEventListener("keydown", onKeyDown, true);
          dom.removeEventListener("click", onClick);
          dom.removeEventListener("compositionend", onCompositionEnd);
        };
      });

      crepe.on((listener) => {
        listener.markdownUpdated((_ctx, markdown) => {
          changeRef.current(markdown);
        });
      });
    });

    return () => {
      cancelled = true;
      teardown?.();
      viewRef.current = null;
      setSuggest(null);
      void crepe.destroy();
      root.innerHTML = "";
    };
    // 只在笔记 id / 初值变化时重建；editable 走独立 effect。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [noteId]);

  // 可编辑性单独同步（预览模式切换不重建编辑器）。
  useEffect(() => {
    const view = viewRef.current;
    if (view) syncEditable(view, editable);
  }, [editable]);

  return (
    <div className="relative min-h-0 flex-1" onCopyCapture={handleCopy}>
      <div ref={rootRef} className="prism-note-editor h-full overflow-y-auto" />
      {suggest && (
        <ul
          className="absolute z-30 max-h-56 w-64 overflow-y-auto rounded-md border border-border bg-popover p-1 text-xs shadow-md"
          style={{ left: suggest.x, top: suggest.y + 6 }}
        >
          {suggest.items.map((note, index) => (
            <li key={note.id}>
              <button
                type="button"
                // mousedown 先于 blur，避免点选丢失
                onMouseDown={(event) => {
                  event.preventDefault();
                  applySuggest(note);
                }}
                className={
                  index === suggest.index
                    ? "flex w-full items-center gap-1.5 rounded-sm bg-brand-soft px-2 py-1 text-left"
                    : "flex w-full items-center gap-1.5 rounded-sm px-2 py-1 text-left hover:bg-muted"
                }
              >
                <span className="truncate">{note.title || "无标题笔记"}</span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/** 图片 URL 是否是「后端能取到字节」的那两种：笔记协议图片、内联 data URL。 */
function copyableImageSrc(src: string): string | null {
  const trimmed = src.trim();
  if (trimmed.startsWith("data:image/")) return trimmed;
  if (!trimmed.startsWith(NOTE_IMAGE_URL_PREFIX)) return null;
  const name = trimmed.slice(NOTE_IMAGE_URL_PREFIX.length).split(/[?#]/)[0];
  return NOTE_IMAGE_NAME.test(name) ? trimmed : null;
}

/** ProseMirror 里选中了一张图片节点：点图片选中整块后右键复制就是这种情况。 */
function selectedNoteImageSrc(selection: EditorState["selection"]): string | null {
  if (!(selection instanceof NodeSelection)) return null;
  const node = selection.node;
  // Crepe 的图片块是 `image-block`，行内图片是 `image`，两种都要认。
  if (!node.type.name.startsWith("image")) return null;
  const src = node.attrs.src;
  return typeof src === "string" ? copyableImageSrc(src) : null;
}

/**
 * DOM 侧兜底：拖选 / 双击选中的内容里**恰好只有一张**笔记图片、且没有文字。
 * 与 `selectedNoteImageSrc` 互补——不产生 NodeSelection 的那种选中只在这里命中。
 */
function selectedNoteImageSrcFromDom(): string | null {
  const selection = window.getSelection();
  if (!selection || selection.isCollapsed || selection.rangeCount === 0) return null;
  const fragment = selection.getRangeAt(0).cloneContents();
  if ((fragment.textContent ?? "").trim()) return null;
  const images = fragment.querySelectorAll("img");
  if (images.length !== 1) return null;
  return copyableImageSrc(images[0].getAttribute("src") ?? "");
}
