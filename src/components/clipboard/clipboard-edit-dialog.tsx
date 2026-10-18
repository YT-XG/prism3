import { useEffect, useState, type KeyboardEvent } from "react";

import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Textarea } from "@/components/ui/textarea";
import { ipc } from "@/lib/ipc";

interface ClipboardEditDialogProps {
  /** 要编辑的记录 id；`null` 表示关闭（主窗口与快捷小窗共用同一个弹窗）。 */
  id: number | null;
  /** 关闭回调：快捷小窗用它把临时接管的键盘焦点还回去。 */
  onClose: () => void;
}

/**
 * 文本记录编辑弹窗（主窗口剪贴板页与快捷小窗剪贴板页共用）。
 *
 * 只改**这条历史记录**：不写回系统剪贴板、也不影响别的记录。保存成功后历史
 * `revision` 变化，列表自己会重查一遍（见 `use-clipboard-list.ts`），
 * 因此这里除了关窗不需要通知谁。
 *
 * 正文按需拉取：列表里只有预览（最长 200 字），点开才 `get_clipboard_text` 取全文。
 * Ctrl/Cmd + Enter 直接保存。
 */
export function ClipboardEditDialog({ id, onClose }: ClipboardEditDialogProps) {
  const [text, setText] = useState("");
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  /** 失败原因就地展示：快捷小窗里 toast 会随面板一起消失，只靠 toast 等于没提示。 */
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (id === null) return;

    let active = true;
    setLoading(true);
    setError(null);
    setText("");
    void ipc
      .getClipboardText(id)
      .then((next) => {
        if (active) setText(next);
      })
      .catch((cause) => {
        if (active) setError(`读取文本失败：${String(cause)}`);
      })
      .finally(() => {
        if (active) setLoading(false);
      });

    return () => {
      active = false;
    };
  }, [id]);

  const save = async () => {
    if (id === null || saving) return;
    setSaving(true);
    setError(null);
    try {
      await ipc.updateClipboardText(id, text);
      toast.success("已保存这条记录");
      onClose();
    } catch (cause) {
      setError(`保存失败：${String(cause)}`);
    } finally {
      setSaving(false);
    }
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if ((event.ctrlKey || event.metaKey) && event.key === "Enter") {
      event.preventDefault();
      void save();
    }
  };

  return (
    <Dialog
      open={id !== null}
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>编辑文本记录</DialogTitle>
          <DialogDescription>
            只修改这条历史记录的正文，不会写回系统剪贴板。Ctrl / Cmd + Enter 保存。
          </DialogDescription>
        </DialogHeader>

        <Textarea
          value={text}
          autoFocus
          spellCheck={false}
          disabled={loading || saving}
          onChange={(event) => setText(event.target.value)}
          onKeyDown={handleKeyDown}
          className="max-h-[50vh] min-h-40 font-mono text-xs"
        />

        <p className="text-xs text-muted-foreground">{text.length} 字</p>

        {error && <p className="text-xs text-destructive">{error}</p>}

        <DialogFooter>
          <Button variant="outline" onClick={onClose} disabled={saving}>
            取消
          </Button>
          <Button onClick={() => void save()} disabled={loading || saving}>
            {saving ? "保存中…" : "保存"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
