import { ExternalLink, Pencil, Plus, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
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
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { useCustomApps } from "@/hooks/use-app-catalog";
import {
  CUSTOM_TINTS,
  customSize,
  customWindowId,
  pickTint,
  type CustomApp,
} from "@/lib/apps";
import { cn } from "@/lib/utils";
import { useAppStore } from "@/stores/app-store";
import { useWindowStore } from "@/stores/window-store";

/** 常用图标：比做 emoji 选择器省事，也足够覆盖「链接 / 文档 / 工具」这几类。 */
const EMOJI_CHOICES = ["🔗", "🌐", "📌", "📝", "📚", "📊", "🎨", "🎵", "🧩", "🛠", "💡", "🎮"];

const newId = () =>
  Math.random().toString(36).slice(2, 10) + Date.now().toString(36).slice(-4);

/**
 * 「创建应用」：把一个网址或一段自写 HTML 变成桌面上的应用。
 *
 * 一个对话框同时管新建、编辑和清单——不做二级页面，
 * 因为这件事本来就轻：起名、选图标、填地址/HTML，存下来就出现在图标坞里。
 */
export function CustomAppDialog() {
  const open = useAppStore((state) => state.customAppDialogOpen);
  const editingId = useAppStore((state) => state.editingCustomAppId);
  const openDialog = useAppStore((state) => state.openCustomAppDialog);
  const close = useAppStore((state) => state.closeCustomAppDialog);
  const saveConfig = useAppStore((state) => state.saveConfig);

  const apps = useCustomApps();
  const editing = editingId ? apps.find((item) => item.id === editingId) ?? null : null;

  const [name, setName] = useState("");
  const [icon, setIcon] = useState("🔗");
  const [kind, setKind] = useState<CustomApp["kind"]>("link");
  const [url, setUrl] = useState("");
  const [html, setHtml] = useState("");
  const [tint, setTint] = useState<[string, string]>(CUSTOM_TINTS[0]);
  const [confirmId, setConfirmId] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  // 每次打开（或切换编辑对象）把表单重置成对应状态
  useEffect(() => {
    if (!open) return;
    setName(editing?.name ?? "");
    setIcon(editing?.icon ?? "🔗");
    setKind(editing?.kind ?? "link");
    setUrl(editing?.url ?? "");
    setHtml(editing?.html ?? "");
    setTint(editing?.tint ?? pickTint(editing?.id ?? newId()));
    setConfirmId(null);
  }, [open, editing]);

  const submit = async () => {
    const trimmed = name.trim();
    if (!trimmed) {
      toast.error("先给应用起个名字");
      return;
    }
    if (kind === "link" && !/^https?:\/\//i.test(url.trim())) {
      toast.error("链接要以 http:// 或 https:// 开头");
      return;
    }
    if (kind === "html" && !html.trim()) {
      toast.error("HTML 内容不能为空");
      return;
    }

    const next: CustomApp = {
      id: editing?.id ?? newId(),
      name: trimmed,
      icon: icon.trim() || "📦",
      kind,
      url: kind === "link" ? url.trim() : "",
      html: kind === "html" ? html : "",
      tint,
    };

    setSaving(true);
    try {
      await saveConfig({
        customApps: editing
          ? apps.map((item) => (item.id === editing.id ? next : item))
          : [...apps, next],
      });
      toast.success(editing ? "应用已更新" : "应用已创建", {
        description: "图标坞与开始菜单里已经能看到了",
      });
      close();
    } finally {
      setSaving(false);
    }
  };

  const remove = async (id: string) => {
    await saveConfig({ customApps: apps.filter((item) => item.id !== id) });
    setConfirmId(null);
    if (editingId === id) close();
    toast.success("应用已删除");
  };

  const launch = (app: CustomApp) => {
    useWindowStore.getState().open(customWindowId(app.id), customSize(app.kind));
  };

  return (
    <Dialog open={open} onOpenChange={(next) => (next ? openDialog(editingId) : close())}>
      <DialogContent className="max-h-[86vh] gap-0 overflow-y-auto sm:max-w-[620px]">
        <DialogHeader>
          <DialogTitle>{editing ? "编辑应用" : "创建应用"}</DialogTitle>
          <DialogDescription>
            一个网址，或者一段自己写的 HTML——保存后就是桌面上的一个应用。
          </DialogDescription>
        </DialogHeader>

        <div className="mt-4 flex flex-col gap-4">
          <div className="flex flex-col gap-2">
            <Label htmlFor="custom-app-name">名称</Label>
            <Input
              id="custom-app-name"
              value={name}
              placeholder="例如：公司内网、番茄钟"
              maxLength={40}
              onChange={(event) => setName(event.target.value)}
            />
          </div>

          <div className="flex flex-col gap-2">
            <div className="flex items-baseline justify-between">
              <Label>类型</Label>
              <span className="text-[11px] text-muted-foreground">
                {kind === "link" ? "交给系统浏览器打开" : "在沙箱 iframe 里运行"}
              </span>
            </div>
            <div className="flex gap-1 rounded-lg bg-muted/60 p-1">
              {(
                [
                  { value: "link", label: "链接" },
                  { value: "html", label: "自定义 HTML" },
                ] as const
              ).map((option) => (
                <button
                  key={option.value}
                  type="button"
                  aria-pressed={kind === option.value}
                  onClick={() => setKind(option.value)}
                  className={cn(
                    "flex-1 rounded-md px-2 py-1.5 text-xs transition-colors",
                    kind === option.value
                      ? "bg-card font-medium text-foreground shadow-[0_1px_2px_rgba(16,40,80,0.08)]"
                      : "text-muted-foreground hover:text-foreground",
                  )}
                >
                  {option.label}
                </button>
              ))}
            </div>
          </div>

          {kind === "link" ? (
            <div className="flex flex-col gap-2">
              <Label htmlFor="custom-app-url">网址</Label>
              <Input
                id="custom-app-url"
                value={url}
                placeholder="https://example.com"
                inputMode="url"
                onChange={(event) => setUrl(event.target.value)}
              />
            </div>
          ) : (
            <div className="flex flex-col gap-2">
              <div className="flex items-baseline justify-between">
                <Label htmlFor="custom-app-html">HTML</Label>
                <span className="text-[11px] text-muted-foreground">
                  支持 &lt;script&gt;、内联样式；不联网也能跑
                </span>
              </div>
              <Textarea
                id="custom-app-html"
                value={html}
                rows={8}
                spellCheck={false}
                placeholder={`<!doctype html>\n<html>\n  <body style="font-family: system-ui; padding: 24px">\n    <h1>你好，Prism3</h1>\n  </body>\n</html>`}
                className="font-mono text-[12px]"
                onChange={(event) => setHtml(event.target.value)}
              />
            </div>
          )}

          <div className="flex flex-col gap-2">
            <Label>图标与配色</Label>
            <div className="flex flex-wrap items-center gap-2">
              {EMOJI_CHOICES.map((value) => (
                <button
                  key={value}
                  type="button"
                  aria-label={value}
                  aria-pressed={icon === value}
                  onClick={() => setIcon(value)}
                  className={cn(
                    "flex size-8 items-center justify-center rounded-lg text-[16px] ring-1 transition",
                    icon === value
                      ? "bg-brand/10 ring-brand/40"
                      : "ring-transparent hover:bg-muted/60",
                  )}
                >
                  {value}
                </button>
              ))}
              <Input
                value={icon}
                onChange={(event) => setIcon(event.target.value)}
                aria-label="自定义图标"
                maxLength={4}
                className="h-8 w-14 text-center text-[14px]"
              />
            </div>

            <div className="mt-1 flex items-center gap-2">
              {CUSTOM_TINTS.map(([from, to]) => {
                const selected = tint[0] === from && tint[1] === to;
                return (
                  <button
                    key={from}
                    type="button"
                    aria-label="图标底色"
                    aria-pressed={selected}
                    onClick={() => setTint([from, to])}
                    className={cn(
                      "size-7 rounded-lg ring-2 ring-offset-2 ring-offset-background transition",
                      selected ? "ring-foreground/50" : "ring-transparent hover:ring-border",
                    )}
                    style={{ backgroundImage: `linear-gradient(150deg, ${from}, ${to})` }}
                  />
                );
              })}
            </div>
          </div>

          {apps.length > 0 ? (
            <div className="flex flex-col gap-2">
              <div className="flex items-baseline justify-between">
                <Label>已创建</Label>
                <span className="text-[11px] text-muted-foreground">{apps.length} 个</span>
              </div>
              <ul className="flex flex-col gap-1">
                {apps.map((app) => (
                  <li
                    key={app.id}
                    className="flex items-center gap-2 rounded-lg border border-border/50 bg-muted/20 px-2 py-1.5"
                  >
                    <span
                      className="flex size-7 shrink-0 items-center justify-center rounded-lg text-[14px] text-white"
                      style={{
                        backgroundImage: `linear-gradient(150deg, ${app.tint[0]}, ${app.tint[1]})`,
                      }}
                    >
                      {app.icon}
                    </span>
                    <button
                      type="button"
                      onClick={() => launch(app)}
                      className="min-w-0 flex-1 truncate text-left text-xs hover:underline"
                      title={app.kind === "link" ? app.url : "打开自定义 HTML"}
                    >
                      {app.name}
                    </button>
                    <Button
                      variant="ghost"
                      size="icon-xs"
                      aria-label={`编辑 ${app.name}`}
                      onClick={() => openDialog(app.id)}
                    >
                      <Pencil />
                    </Button>
                    {confirmId === app.id ? (
                      <Button
                        variant="destructive"
                        size="xs"
                        onClick={() => void remove(app.id)}
                        onBlur={() => setConfirmId(null)}
                      >
                        确认删除
                      </Button>
                    ) : (
                      <Button
                        variant="ghost"
                        size="icon-xs"
                        aria-label={`删除 ${app.name}`}
                        onClick={() => setConfirmId(app.id)}
                      >
                        <Trash2 />
                      </Button>
                    )}
                  </li>
                ))}
              </ul>
            </div>
          ) : null}
        </div>

        <DialogFooter className="mt-5">
          {editing ? (
            <Button
              variant="ghost"
              className="mr-auto"
              onClick={() => openDialog(null)}
            >
              <Plus data-icon="inline-start" />
              新建一个
            </Button>
          ) : null}
          <Button variant="outline" onClick={close}>
            取消
          </Button>
          <Button onClick={() => void submit()} disabled={saving}>
            {editing ? "保存修改" : "创建应用"}
          </Button>
        </DialogFooter>

        {apps.length === 0 ? (
          <p className="mt-3 flex items-center gap-1.5 text-[11px] text-muted-foreground">
            <ExternalLink className="size-3" aria-hidden />
            链接型应用不内嵌网页，点开会交给系统浏览器。
          </p>
        ) : null}
      </DialogContent>
    </Dialog>
  );
}
