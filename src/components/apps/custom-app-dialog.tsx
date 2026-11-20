import { ExternalLink, Pencil, Plus, Trash2 } from "lucide-react";
import { useEffect, useRef, useState } from "react";
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
import { CUSTOM_TINTS, customSize, customWindowId, pickTint, type CustomApp } from "@/lib/apps";
import { cn } from "@/lib/utils";
import { useAppStore } from "@/stores/app-store";
import { useWindowStore } from "@/stores/window-store";

/** 常用图标：比做 emoji 选择器省事，也足够覆盖「链接 / 文档 / 工具」这几类。 */
const EMOJI_CHOICES = ["🔗", "🌐", "📌", "📝", "📚", "📊", "🎨", "🎵", "🧩", "🛠", "💡", "🎮"];

const SAMPLE_HTML = `<!doctype html>
<html>
  <body style="font-family: system-ui; padding: 32px">
    <h1 style="margin: 0 0 8px">你好，Prism3</h1>
    <p style="color: #64748b">这是一段自己写的 HTML，跑在沙箱 iframe 里。</p>
  </body>
</html>`;

const newId = () => Math.random().toString(36).slice(2, 10) + Date.now().toString(36).slice(-4);

/**
 * 「创建应用」：把一个网址或一段自写 HTML 变成桌面上的应用。
 *
 * 版式照参考系统：**左边表单、右边代码**。代码区独占一列，
 * 竖着能写十几行也不用挤在表单下面滚；宽屏时左右并排，窄屏自动叠起来。
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
  const nameRef = useRef<HTMLInputElement>(null);

  // 每次打开（或切换编辑对象）把表单重置成对应状态
  useEffect(() => {
    if (!open) return;
    setName(editing?.name ?? "");
    setIcon(editing?.icon ?? "🔗");
    setKind(editing?.kind ?? "link");
    setUrl(editing?.url ?? "");
    setHtml(editing?.html ?? SAMPLE_HTML);
    setTint(editing?.tint ?? pickTint(editing?.id ?? newId()));
    setConfirmId(null);
  }, [open, editing]);

  const submit = async () => {
    const trimmed = name.trim();
    if (!trimmed) {
      toast.error("先给应用起个名字");
      nameRef.current?.focus();
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
      <DialogContent className="grid max-h-[88vh] grid-rows-[auto_auto_minmax(0,1fr)_auto] gap-0 overflow-hidden p-0 sm:max-w-[900px]">
        <DialogHeader className="shrink-0 px-6 pt-5 pb-1">
          <DialogTitle>{editing ? "编辑应用" : "创建应用"}</DialogTitle>
          <DialogDescription>
            一个网址，或者一段自己写的 HTML——保存后就是桌面上的一个应用。
          </DialogDescription>
        </DialogHeader>

        {/* 类型当作顶部两个页签：切换即换掉右列的内容 */}
        <div className="shrink-0 px-6 pt-3">
          <div className="flex gap-1 rounded-lg bg-muted/60 p-1">
            {(
              [
                { value: "link", label: "网站应用" },
                { value: "html", label: "自编应用（HTML）" },
              ] as const
            ).map((option) => (
              <button
                key={option.value}
                type="button"
                aria-pressed={kind === option.value}
                onClick={() => setKind(option.value)}
                className={cn(
                  "flex-1 rounded-md px-3 py-1.5 text-xs transition-colors",
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

        <div className="grid min-h-0 grid-cols-1 gap-5 overflow-y-auto px-6 py-4 sm:grid-cols-[248px_minmax(0,1fr)]">
          {/* 左列：表单 */}
          <div className="flex min-w-0 flex-col gap-4">
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="custom-app-name">应用名称</Label>
              <Input
                id="custom-app-name"
                ref={nameRef}
                value={name}
                placeholder="例如：公司内网、番茄钟"
                maxLength={40}
                onChange={(event) => setName(event.target.value)}
              />
            </div>

            <div className="flex flex-col gap-1.5">
              <Label>图标</Label>
              <div className="grid grid-cols-6 gap-1">
                {EMOJI_CHOICES.map((value) => (
                  <button
                    key={value}
                    type="button"
                    aria-label={value}
                    aria-pressed={icon === value}
                    onClick={() => setIcon(value)}
                    className={cn(
                      "flex h-8 items-center justify-center rounded-lg text-[15px] ring-1 transition",
                      icon === value
                        ? "bg-brand/10 ring-brand/40"
                        : "ring-transparent hover:bg-muted/60",
                    )}
                  >
                    {value}
                  </button>
                ))}
              </div>
              <Input
                value={icon}
                onChange={(event) => setIcon(event.target.value)}
                aria-label="自定义图标"
                placeholder="也可以粘贴任意 emoji"
                maxLength={4}
                className="h-7 text-center text-[13px]"
              />
            </div>

            <div className="flex flex-col gap-1.5">
              <Label>图标颜色</Label>
              <div className="grid grid-cols-4 gap-2">
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
                        "h-7 rounded-lg ring-2 ring-offset-2 ring-offset-background transition",
                        selected ? "ring-foreground/50" : "ring-transparent hover:ring-border",
                      )}
                      style={{ backgroundImage: `linear-gradient(150deg, ${from}, ${to})` }}
                    />
                  );
                })}
              </div>
            </div>

            {apps.length > 0 ? (
              <div className="flex min-h-0 flex-col gap-1.5">
                <div className="flex items-baseline justify-between">
                  <Label>已创建</Label>
                  <span className="text-[11px] text-muted-foreground">{apps.length} 个</span>
                </div>
                <ul className="flex max-h-[168px] flex-col gap-1 overflow-y-auto pr-0.5">
                  {apps.map((app) => (
                    <li
                      key={app.id}
                      className="flex items-center gap-1.5 rounded-lg border border-border/50 bg-muted/20 px-1.5 py-1"
                    >
                      <span
                        className="flex size-6 shrink-0 items-center justify-center rounded-md text-[12px] text-white"
                        style={{
                          backgroundImage: `linear-gradient(150deg, ${app.tint[0]}, ${app.tint[1]})`,
                        }}
                      >
                        {app.icon}
                      </span>
                      <button
                        type="button"
                        onClick={() => launch(app)}
                        className="min-w-0 flex-1 truncate text-left text-[11.5px] hover:underline"
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
                          确认
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

          {/* 右列：链接地址 / HTML 代码 */}
          {kind === "link" ? (
            <div className="flex min-h-0 flex-col gap-2">
              <Label htmlFor="custom-app-url">网站地址</Label>
              <Input
                id="custom-app-url"
                value={url}
                placeholder="https://example.com"
                inputMode="url"
                onChange={(event) => setUrl(event.target.value)}
              />

              <div className="mt-2 flex min-h-[240px] flex-col items-center justify-center gap-3 rounded-xl border border-dashed bg-muted/15 px-6 text-center">
                <span
                  className="flex size-14 items-center justify-center rounded-[16px] text-[26px] text-white shadow-[0_10px_24px_-14px_rgba(16,40,80,.7)]"
                  style={{
                    backgroundImage: `linear-gradient(150deg, ${tint[0]}, ${tint[1]})`,
                  }}
                >
                  {icon || "🔗"}
                </span>
                <div className="min-w-0">
                  <p className="truncate text-sm font-medium">{name || "应用名称"}</p>
                  <p className="mt-1 truncate text-[11px] text-muted-foreground">
                    {url || "保存后点图标，交给系统浏览器打开"}
                  </p>
                </div>
                <p className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
                  <ExternalLink className="size-3" aria-hidden />
                  默认在窗口内嵌打开，也可点右上角按钮交给系统浏览器
                </p>
              </div>
            </div>
          ) : (
            <div className="flex min-h-0 flex-col gap-2">
              <div className="flex items-baseline justify-between">
                <Label htmlFor="custom-app-html">HTML 内容</Label>
                <span className="text-[11px] text-muted-foreground">
                  &lt;script&gt; 可运行 · 拿不到外壳与 IPC
                </span>
              </div>
              <Textarea
                id="custom-app-html"
                value={html}
                spellCheck={false}
                placeholder={SAMPLE_HTML}
                onChange={(event) => setHtml(event.target.value)}
                className="h-[min(46vh,420px)] min-h-[280px] resize-none rounded-xl border border-border/60 bg-[color-mix(in_srgb,var(--foreground)_4%,var(--background))] font-mono text-[12.5px] leading-[1.65]"
              />
              <p className="text-[11px] leading-relaxed text-muted-foreground">
                写好 HTML 就能用：cwd 里不联网也能跑；要预览可以先保存，再从图标坞点开。
              </p>
            </div>
          )}
        </div>

        <DialogFooter className="shrink-0 border-t px-6 py-3">
          {editing ? (
            <Button variant="ghost" className="mr-auto" onClick={() => openDialog(null)}>
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
      </DialogContent>
    </Dialog>
  );
}
