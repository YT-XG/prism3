import { useEffect, useState, type KeyboardEvent } from "react";

import { info } from "@tauri-apps/plugin-log";
import { open } from "@tauri-apps/plugin-dialog";
import {
  ClipboardList,
  Eraser,
  FileDown,
  FileUp,
  FolderSearch,
  ImagePlus,
  Monitor,
  Moon,
  NotebookPen,
  RotateCcw,
  Sun,
  Trash2,
} from "lucide-react";
import { toast } from "sonner";

import { UpdateCard } from "@/components/app/update-card";
import { ClipboardStatus } from "@/components/clipboard/clipboard-status";
import { SearchIndexStatus } from "@/components/search/search-status";
import {
  ACCENT_COLORS,
  presetGradient,
  resolveAccent,
  resolvePreset,
  THEME_PRESETS,
} from "@/components/theme-provider";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useClipboardState } from "@/hooks/use-clipboard-state";
import { useFileIndexState } from "@/hooks/use-file-index-state";
import { ipc, type AppConfig, type NoteEditorMode, type Theme } from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { useAppStore } from "@/stores/app-store";

/** 保留天数的可填范围（0 = 永久保留）。 */
const MAX_RETENTION_DAYS = 3650;

const themes: { value: Theme; label: string; icon: typeof Sun }[] = [
  { value: "system", label: "跟随系统", icon: Monitor },
  { value: "light", label: "浅色", icon: Sun },
  { value: "dark", label: "深色", icon: Moon },
];

/** 设置面板：配置读写的端到端示例。 */
export function SettingsPanel() {
  const config = useAppStore((state) => state.config);
  const saveConfig = useAppStore((state) => state.saveConfig);
  const resetConfig = useAppStore((state) => state.resetConfig);
  const indexState = useFileIndexState(true);
  const clipboardState = useClipboardState();
  const setActiveView = useAppStore((state) => state.setActiveView);
  const [notesFeedback, setNotesFeedback] = useState<string | null>(null);
  const [notesBusy, setNotesBusy] = useState(false);

  if (!config) {
    return <p className="text-sm text-muted-foreground">正在读取配置…</p>;
  }

  const apply = async (patch: Partial<AppConfig>) => {
    try {
      await saveConfig(patch);
      await info(`配置已更新：${JSON.stringify(patch)}`);
      toast.success("设置已保存");
    } catch (cause) {
      toast.error(`保存失败：${String(cause)}`);
    }
  };

  const restoreDefaults = async () => {
    try {
      await resetConfig();
      await info("配置已恢复默认");
      toast.success("已恢复默认设置");
    } catch (cause) {
      toast.error(`恢复失败：${String(cause)}`);
    }
  };

  const rebuildIndex = async () => {
    try {
      await ipc.rebuildFileIndex();
      toast.success("已开始重建索引");
    } catch (cause) {
      toast.error(`重建失败：${String(cause)}`);
    }
  };

  const exportNotes = async () => {
    const dir = await open({ directory: true, title: "选择速记导出目录" });
    if (typeof dir !== "string") return;
    setNotesBusy(true);
    setNotesFeedback("导出中…");
    try {
      const report = await ipc.exportNotes(dir);
      setNotesFeedback(
        `已导出 ${report.exported} 篇（图片 ${report.images} 张）${
          report.failed.length ? `，${report.failed.length} 条失败` : ""
        }`,
      );
    } catch (cause) {
      setNotesFeedback(`导出失败：${String(cause)}`);
    } finally {
      setNotesBusy(false);
    }
  };

  const importNotes = async () => {
    const dir = await open({ directory: true, title: "选择速记导入目录" });
    if (typeof dir !== "string") return;
    setNotesBusy(true);
    setNotesFeedback("导入中…");
    try {
      const report = await ipc.importNotes(dir);
      setNotesFeedback(
        `导入 ${report.imported} 篇，跳过 ${report.skipped}${
          report.failed.length ? `，失败 ${report.failed.length}` : ""
        }`,
      );
    } catch (cause) {
      setNotesFeedback(`导入失败：${String(cause)}`);
    } finally {
      setNotesBusy(false);
    }
  };

  const gcNoteImages = async () => {
    setNotesBusy(true);
    try {
      const dry = await ipc.gcOrphanNoteImages(true);
      if (dry.orphanFiles.length === 0) {
        setNotesFeedback(`没有孤儿图片（共 ${dry.totalFiles} 张，均被引用）`);
        return;
      }
      if (
        !window.confirm(
          `发现 ${dry.orphanFiles.length} 张未被任何笔记引用的图片，永久删除它们？`,
        )
      ) {
        setNotesFeedback(null);
        return;
      }
      const done = await ipc.gcOrphanNoteImages(false);
      setNotesFeedback(`已清理 ${done.removed} 张孤儿图片${done.failed ? `，${done.failed} 张失败` : ""}`);
    } catch (cause) {
      setNotesFeedback(`清理失败：${String(cause)}`);
    } finally {
      setNotesBusy(false);
    }
  };

  const cancelIndex = async () => {
    try {
      await ipc.cancelFileIndex();
    } catch (cause) {
      toast.error(`取消失败：${String(cause)}`);
    }
  };

  const pickWallpaper = async () => {
    try {
      const picked = await open({
        multiple: false,
        directory: false,
        filters: [{ name: "图片", extensions: ["png", "jpg", "jpeg", "webp", "bmp", "gif"] }],
      });
      if (typeof picked === "string") await apply({ wallpaper: picked });
    } catch (cause) {
      toast.error(`选择壁纸失败：${String(cause)}`);
    }
  };

  return (
    <div className="flex flex-col gap-4">
      <div>
        <h1 className="font-heading text-lg font-medium">设置</h1>
        <p className="text-sm text-muted-foreground">
          这里的改动会通过 IPC 交给 Rust 侧持久化。
        </p>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>外观</CardTitle>
          <CardDescription>
            三轴可调：模式（亮 / 暗 / 跟随系统）、配色（背景与玻璃染色）、强调色（按钮与焦点环）。
          </CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col gap-5">
          <Tabs
            value={config.theme}
            onValueChange={(value) => void apply({ theme: value as Theme })}
          >
            <TabsList>
              {themes.map(({ value, label, icon: Icon }) => (
                <TabsTrigger key={value} value={value}>
                  <Icon data-icon="inline-start" />
                  {label}
                </TabsTrigger>
              ))}
            </TabsList>
          </Tabs>

          <div className="flex flex-col gap-2">
            <div className="flex items-baseline justify-between">
              <Label>配色</Label>
              <span className="text-xs text-muted-foreground">只影响背景光晕与玻璃染色</span>
            </div>
            <div className="grid grid-cols-5 gap-2">
              {THEME_PRESETS.map((preset) => {
                const selected = resolvePreset(config.themePreset) === preset.id;
                return (
                  <button
                    key={preset.id}
                    type="button"
                    aria-pressed={selected}
                    onClick={() => void apply({ themePreset: preset.id })}
                    className={cn(
                      "group flex flex-col items-center gap-1.5 rounded-lg p-1.5 transition-colors",
                      selected ? "bg-brand-soft" : "hover:bg-muted",
                    )}
                  >
                    <span
                      className="h-8 w-full rounded-md ring-1 ring-black/5 dark:ring-white/10"
                      style={{ backgroundImage: presetGradient(preset) }}
                    />
                    <span
                      className={cn(
                        "text-[11px]",
                        selected ? "font-medium text-foreground" : "text-muted-foreground",
                      )}
                    >
                      {preset.label}
                    </span>
                  </button>
                );
              })}
            </div>
          </div>

          <div className="flex flex-col gap-2">
            <div className="flex items-baseline justify-between">
              <Label>强调色</Label>
              <span className="text-xs text-muted-foreground">按钮、焦点环与选中态</span>
            </div>
            <div className="flex flex-wrap items-center gap-2">
              {ACCENT_COLORS.map(({ value, label }) => {
                const selected = resolveAccent(config.accentColor) === value;
                return (
                  <button
                    key={value}
                    type="button"
                    aria-label={label}
                    aria-pressed={selected}
                    title={label}
                    onClick={() => void apply({ accentColor: value })}
                    className={cn(
                      "size-7 rounded-full ring-2 ring-offset-2 ring-offset-background transition",
                      selected ? "ring-foreground/60" : "ring-transparent hover:ring-border",
                    )}
                    style={{ backgroundColor: value }}
                  />
                );
              })}

              <label
                className="relative flex size-7 cursor-pointer items-center justify-center rounded-full ring-1 ring-border transition hover:ring-foreground/40"
                title="自定义强调色"
              >
                <span
                  aria-hidden="true"
                  className="size-5 rounded-full"
                  style={{
                    background:
                      "conic-gradient(#f43f5e, #f59e0b, #10b981, #06b6d4, #5b5bf5, #8b5cf6, #f43f5e)",
                  }}
                />
                <input
                  type="color"
                  aria-label="自定义强调色"
                  value={resolveAccent(config.accentColor)}
                  onChange={(event) => void apply({ accentColor: event.target.value })}
                  className="absolute inset-0 size-full cursor-pointer opacity-0"
                />
              </label>
            </div>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>壁纸与玻璃</CardTitle>
          <CardDescription>
            选一张图片做底；有壁纸时亮/暗主题都用半透明玻璃卡片，亮色保留深色文字。
          </CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col gap-5">
          <div className="flex items-center justify-between gap-3">
            <div className="flex min-w-0 flex-col gap-0.5">
              <Label>当前壁纸</Label>
              <span className="truncate text-xs text-muted-foreground" title={config.wallpaper}>
                {config.wallpaper || "未设置（使用渐变底）"}
              </span>
            </div>
            <div className="flex shrink-0 items-center gap-2">
              <Button variant="outline" size="sm" onClick={() => void pickWallpaper()}>
                <ImagePlus data-icon="inline-start" />
                选择图片
              </Button>
              {config.wallpaper && (
                <Button variant="ghost" size="sm" onClick={() => void apply({ wallpaper: "" })}>
                  <Trash2 data-icon="inline-start" />
                  清除
                </Button>
              )}
            </div>
          </div>

          <RangeRow
            label="背景模糊"
            hint="整层壁纸一起模糊；卡片只做半透明，不逐张开 backdrop-filter"
            value={config.wallpaperBlur}
            min={0}
            max={40}
            suffix="px"
            disabled={!config.wallpaper}
            onCommit={(value) => void apply({ wallpaperBlur: value })}
          />

          <RangeRow
            label="蒙版强度"
            hint="调整壁纸与卡片的对比度（亮色蒙白 / 暗色压暗）"
            value={config.wallpaperVeil}
            min={0}
            max={90}
            suffix="%"
            disabled={!config.wallpaper}
            onCommit={(value) => void apply({ wallpaperVeil: value })}
          />

          <RangeRow
            label="玻璃不透明度"
            hint={
              config.theme === "light" && !config.wallpaper
                ? "亮色 + 无壁纸时卡片保持实心白；设置壁纸后这里控制玻璃透明度"
                : "越低越透出背景；30% 以下文字会开始难读"
            }
            value={config.glassOpacity}
            min={30}
            max={100}
            suffix="%"
            disabled={config.theme === "light" && !config.wallpaper}
            onCommit={(value) => void apply({ glassOpacity: value })}
          />
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>窗口行为</CardTitle>
          <CardDescription>关闭主窗口时是否保留后台托盘图标；开机自启默认开启。</CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col gap-3">
          <div className="flex items-center justify-between">
            <Label htmlFor="close-to-tray">关闭窗口时最小化到托盘</Label>
            <Switch
              id="close-to-tray"
              checked={config.closeToTray}
              onCheckedChange={(checked) => void apply({ closeToTray: checked })}
            />
          </div>

          <div className="flex items-center justify-between">
            <div className="flex flex-col gap-0.5">
              <Label htmlFor="auto-start">开机时自动启动</Label>
              <span className="text-xs text-muted-foreground">
                {import.meta.env.DEV
                  ? "开发模式下不注册系统启动项，只有发布版才生效。"
                  : "登录系统后在托盘静默运行，不弹主窗口；点托盘图标可打开。"}
              </span>
            </div>
            <Switch
              id="auto-start"
              checked={config.autoStart}
              onCheckedChange={(checked) => void apply({ autoStart: checked })}
            />
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>快捷小窗</CardTitle>
          <CardDescription>在屏幕右缘显示一条灰线，鼠标移上去展开小面板。</CardDescription>
        </CardHeader>
        <CardContent className="flex items-center justify-between">
          <Label htmlFor="quick-panel">启用右缘快捷小窗</Label>
          <Switch
            id="quick-panel"
            checked={config.quickPanel}
            onCheckedChange={(checked) => void apply({ quickPanel: checked })}
          />
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>组合窗口</CardTitle>
          <CardDescription>
            用全局快捷键唤起一个透明窗口，并排显示快速搜索 / 速记 / 剪贴板 / 项目四张卡片；
            按 Esc、点击窗口外或窗口失焦都会收起。
          </CardDescription>
        </CardHeader>
        <CardContent className="flex items-center justify-between gap-3">
          <div className="flex min-w-0 flex-col gap-0.5">
            <Label htmlFor="hub-shortcut">唤起快捷键</Label>
            <span className="text-xs text-muted-foreground">
              点击输入框后按下组合键即可录制；Backspace 清除（清除后不再注册）。
            </span>
          </div>
          <ShortcutRecorder
            id="hub-shortcut"
            value={config.hubShortcut}
            onCommit={(next) => void apply({ hubShortcut: next })}
          />
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>文件搜索</CardTitle>
          <CardDescription>
            为常用目录建立文件名索引，实现本机毫秒级搜索；不联网、不读取文件内容。
            索引范围在「搜索」页里调整。
          </CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col gap-3">
          <div className="flex items-center justify-between">
            <Label htmlFor="file-search">启用本地文件搜索</Label>
            <Switch
              id="file-search"
              checked={config.fileSearch}
              onCheckedChange={(checked) => void apply({ fileSearch: checked })}
            />
          </div>

          <SearchIndexStatus
            state={indexState}
            onRebuild={() => void rebuildIndex()}
            onCancel={() => void cancelIndex()}
          />

          {config.fileSearch && (
            <div>
              <Button variant="outline" size="sm" onClick={() => setActiveView("search")}>
                <FolderSearch data-icon="inline-start" />
                管理索引范围
              </Button>
            </div>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>剪贴板历史</CardTitle>
          <CardDescription>
            记录复制过的文本与图片，点击一条记录即可回到上一个窗口并粘贴；数据只保存在本机，
            不联网。列表、搜索与编辑在「剪贴板」页；收藏的记录不会被自动清理（按保留天数裁剪 /
            清空历史都不会删掉它，只有对这条记录手动删除才会移除）。
          </CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col gap-3">
          <div className="flex items-center justify-between">
            <Label htmlFor="clipboard-history">启用剪贴板历史</Label>
            <Switch
              id="clipboard-history"
              checked={config.clipboardHistory}
              onCheckedChange={(checked) => void apply({ clipboardHistory: checked })}
            />
          </div>

          <div className="flex items-center justify-between gap-3">
            <div className="flex flex-col gap-0.5">
              <Label htmlFor="clipboard-retention">保留天数</Label>
              <span className="text-xs text-muted-foreground">到期自动清理；填 0 表示永久保留。</span>
            </div>
            <RetentionInput
              value={config.clipboardRetentionDays}
              onCommit={(days) => void apply({ clipboardRetentionDays: days })}
            />
          </div>

          <div className="flex items-center justify-between">
            <div className="flex flex-col gap-0.5">
              <Label htmlFor="clipboard-images">记录图片</Label>
              <span className="text-xs text-muted-foreground">截图等图片体积较大，可单独关掉。</span>
            </div>
            <Switch
              id="clipboard-images"
              checked={config.clipboardCaptureImages}
              onCheckedChange={(checked) => void apply({ clipboardCaptureImages: checked })}
            />
          </div>

          <div className="flex items-center justify-between">
            <div className="flex flex-col gap-0.5">
              <Label htmlFor="clipboard-paste">点击记录时自动粘贴</Label>
              <span className="text-xs text-muted-foreground">
                关闭后点击只把内容复制回系统剪贴板。
              </span>
            </div>
            <Switch
              id="clipboard-paste"
              checked={config.clipboardPasteOnClick}
              onCheckedChange={(checked) => void apply({ clipboardPasteOnClick: checked })}
            />
          </div>

          <ClipboardStatus state={clipboardState} />

          {config.clipboardHistory && (
            <div>
              <Button variant="outline" size="sm" onClick={() => setActiveView("clipboard")}>
                <ClipboardList data-icon="inline-start" />
                打开剪贴板页
              </Button>
            </div>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>速记</CardTitle>
          <CardDescription>
            文件夹、标签、回收站与双链的笔记系统；正文以 Markdown 保存，可整库导出为 .md
            文件（含图片）。完整列表与编辑在「速记」页。
          </CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col gap-3">
          <div className="flex items-center justify-between gap-3">
            <div className="flex flex-col gap-0.5">
              <Label>编辑器模式</Label>
              <span className="text-xs text-muted-foreground">
                实时预览 / 仅预览 / 源码 / 分屏。
              </span>
            </div>
            <Tabs
              value={config.noteEditorMode}
              onValueChange={(value) => void apply({ noteEditorMode: value as NoteEditorMode })}
            >
              <TabsList>
                <TabsTrigger value="wysiwyg">实时</TabsTrigger>
                <TabsTrigger value="preview">预览</TabsTrigger>
                <TabsTrigger value="source">源码</TabsTrigger>
                <TabsTrigger value="split">分屏</TabsTrigger>
              </TabsList>
            </Tabs>
          </div>

          <div className="flex items-center justify-between gap-3">
            <div className="flex flex-col gap-0.5">
              <Label htmlFor="notes-retention">回收站保留天数</Label>
              <span className="text-xs text-muted-foreground">
                到期自动清理回收站；填 0 表示永久保留。
              </span>
            </div>
            <RetentionInput
              id="notes-retention"
              value={config.noteTrashRetentionDays}
              onCommit={(days) => void apply({ noteTrashRetentionDays: days })}
            />
          </div>

          <div className="flex flex-wrap items-center gap-2">
            <Button variant="outline" size="sm" disabled={notesBusy} onClick={() => void exportNotes()}>
              <FileDown data-icon="inline-start" />
              导出为 .md
            </Button>
            <Button variant="outline" size="sm" disabled={notesBusy} onClick={() => void importNotes()}>
              <FileUp data-icon="inline-start" />
              从 .md 导入
            </Button>
            <Button variant="outline" size="sm" disabled={notesBusy} onClick={() => void gcNoteImages()}>
              <Eraser data-icon="inline-start" />
              清理孤儿图片
            </Button>
            <Button variant="outline" size="sm" onClick={() => setActiveView("notes")}>
              <NotebookPen data-icon="inline-start" />
              打开速记页
            </Button>
          </div>

          {notesFeedback && (
            <p className="break-words text-xs text-muted-foreground">{notesFeedback}</p>
          )}
        </CardContent>
      </Card>

      <UpdateCard />

      <Card>
        <CardHeader>
          <CardTitle>重置</CardTitle>
          <CardDescription>把全部设置恢复为默认值。</CardDescription>
        </CardHeader>
        <CardFooter>
          <Button variant="outline" onClick={() => void restoreDefaults()}>
            <RotateCcw data-icon="inline-start" />
            恢复默认设置
          </Button>
        </CardFooter>
      </Card>
    </div>
  );
}

/**
 * 保留天数输入框：本地维护草稿，**失焦或回车才提交**。
 *
 * 每敲一个字符就写配置会把「30」打成「3」→ 立刻裁掉 3 天以前的记录（真实数据丢失），
 * 因此中间态只存在这里，提交时再夹到 0..3650。
 */
function RetentionInput({
  value,
  onCommit,
  id = "clipboard-retention",
}: {
  value: number;
  onCommit: (days: number) => void;
  /** 输入框 id（剪贴板与速记两处复用，避免重复 id）。 */
  id?: string;
}) {
  const [draft, setDraft] = useState(String(value));

  // 外部值变化（恢复默认 / 别的入口改写配置）时把草稿拉回来对齐。
  useEffect(() => setDraft(String(value)), [value]);

  const commit = () => {
    const parsed = Number.parseInt(draft, 10);
    const days = Number.isFinite(parsed)
      ? Math.min(MAX_RETENTION_DAYS, Math.max(0, parsed))
      : value;
    setDraft(String(days));
    if (days !== value) onCommit(days);
  };

  return (
    <Input
      id={id}
      type="number"
      min={0}
      max={MAX_RETENTION_DAYS}
      value={draft}
      onChange={(event) => setDraft(event.target.value)}
      onBlur={commit}
      onKeyDown={(event) => {
        if (event.key === "Enter") {
          event.preventDefault();
          commit();
        }
      }}
      className="h-8 w-24 text-xs"
    />
  );
}

/**
 * 带本地草稿的滑杆：拖动期间只改 UI，**松手 / 键盘抬起 / 失焦才写配置**。
 *
 * 每移动一像素就 `set_config` 会拖着重写整份配置 + 落盘，既卡又没必要。
 */
function RangeRow({
  label,
  hint,
  value,
  min,
  max,
  suffix,
  disabled = false,
  onCommit,
}: {
  label: string;
  hint?: string;
  value: number;
  min: number;
  max: number;
  suffix: string;
  disabled?: boolean;
  onCommit: (value: number) => void;
}) {
  const [draft, setDraft] = useState(value);

  useEffect(() => setDraft(value), [value]);

  const commit = () => {
    if (draft !== value) onCommit(draft);
  };

  return (
    <div className={cn("flex flex-col gap-1", disabled && "opacity-50")}>
      <div className="flex items-baseline justify-between gap-3">
        <Label>{label}</Label>
        <span className="text-xs tabular-nums text-muted-foreground">
          {draft}
          {suffix}
        </span>
      </div>
      <input
        type="range"
        min={min}
        max={max}
        step={1}
        value={draft}
        disabled={disabled}
        aria-label={label}
        onChange={(event) => setDraft(Number(event.target.value))}
        onPointerUp={commit}
        onKeyUp={commit}
        onBlur={commit}
        className="h-1.5 w-full cursor-pointer appearance-none rounded-full bg-muted accent-[var(--brand)]"
      />
      {hint && <span className="text-xs text-muted-foreground">{hint}</span>}
    </div>
  );
}

/** 录制的命名键：与 Rust 侧 `hotkey.rs` 的解析表保持一致。 */
const NAMED_KEYS: Record<string, string> = {
  " ": "Space",
  Spacebar: "Space",
  Enter: "Enter",
  Tab: "Tab",
  ArrowUp: "Up",
  ArrowDown: "Down",
  ArrowLeft: "Left",
  ArrowRight: "Right",
  Home: "Home",
  End: "End",
  PageUp: "PageUp",
  PageDown: "PageDown",
  Insert: "Insert",
  Delete: "Delete",
};

function normalizeShortcutKey(key: string): string | null {
  if (key.length === 1 && /[a-z0-9]/i.test(key)) return key.toUpperCase();
  if (/^f\d{1,2}$/i.test(key)) return key.toUpperCase();
  return NAMED_KEYS[key] ?? null;
}

/** 把一次按键转换成 `Ctrl+Shift+C`；只按住修饰键时返回 null（继续等待）。 */
function shortcutFromEvent(event: KeyboardEvent<HTMLInputElement>): string | null {
  const modifiers: string[] = [];
  if (event.ctrlKey) modifiers.push("Ctrl");
  if (event.altKey) modifiers.push("Alt");
  if (event.shiftKey) modifiers.push("Shift");
  if (event.metaKey) modifiers.push("Win");
  // 必须带修饰键，避免注册一个单键全局快捷键把正常输入抢走。
  if (modifiers.length === 0) return null;

  const key = normalizeShortcutKey(event.key);
  return key ? [...modifiers, key].join("+") : null;
}

/** 全局快捷键录制框：聚焦即进入录制，按 Backspace 清除（空串 = 不注册）。 */
function ShortcutRecorder({
  id,
  value,
  onCommit,
}: {
  id: string;
  value: string;
  onCommit: (next: string) => void;
}) {
  const [recording, setRecording] = useState(false);
  const [draft, setDraft] = useState(value);

  useEffect(() => setDraft(value), [value]);

  const finish = (next: string) => {
    setRecording(false);
    setDraft(next);
    if (next !== value) onCommit(next);
  };

  return (
    <Input
      id={id}
      readOnly
      value={recording ? "" : draft}
      placeholder={recording ? "请按下组合键…" : "点击录制"}
      onFocus={() => {
        setRecording(true);
        setDraft("");
      }}
      onBlur={() => {
        setRecording(false);
        setDraft(value);
      }}
      onKeyDown={(event) => {
        if (!recording) return;
        // 录制期间不执行输入框的默认行为（Tab 跳焦点 / 空格滚动等）。
        event.preventDefault();
        if (event.key === "Backspace") {
          finish("");
          return;
        }
        if (event.key === "Escape") {
          setRecording(false);
          setDraft(value);
          return;
        }
        const shortcut = shortcutFromEvent(event);
        if (shortcut) finish(shortcut);
      }}
      className="h-8 w-52 font-mono text-xs"
    />
  );
}
