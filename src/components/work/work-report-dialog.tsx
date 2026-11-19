import { useMemo, useState } from "react";

import { Check, Copy } from "lucide-react";
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
import { ipc, type WorkLog } from "@/lib/ipc";
import { buildWorkReport } from "@/lib/work-report";

interface WorkReportDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  logs: WorkLog[];
  from: number;
  to: number;
}

/** 周报预览：按项目分组生成 Markdown，一键复制到剪贴板。 */
export function WorkReportDialog({ open, onOpenChange, logs, from, to }: WorkReportDialogProps) {
  const [copied, setCopied] = useState(false);
  const report = useMemo(() => buildWorkReport(logs, from, to), [logs, from, to]);

  const copy = async () => {
    try {
      await ipc.copyTextToClipboard(report);
      setCopied(true);
      toast.success("周报已复制到剪贴板");
      window.setTimeout(() => setCopied(false), 1600);
    } catch (cause) {
      toast.error(`复制失败：${String(cause)}`);
    }
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        setCopied(false);
        onOpenChange(next);
      }}
    >
      <DialogContent className="sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>本周工作报告</DialogTitle>
          <DialogDescription>
            由本周 {logs.length} 条记录生成，按项目分组；复制后可直接粘进日报 / 周报。
          </DialogDescription>
        </DialogHeader>

        <pre className="max-h-[55vh] overflow-auto rounded-lg bg-muted/40 p-3 text-xs leading-relaxed whitespace-pre-wrap select-text">
          {report}
        </pre>

        <DialogFooter>
          <Button variant="outline" size="sm" onClick={() => onOpenChange(false)}>
            关闭
          </Button>
          <Button size="sm" className="gap-1.5" onClick={() => void copy()}>
            {copied ? <Check className="size-3.5" aria-hidden /> : <Copy className="size-3.5" aria-hidden />}
            复制 Markdown
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
