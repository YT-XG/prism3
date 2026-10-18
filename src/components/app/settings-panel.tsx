import { useEffect, useState } from "react";

import { info } from "@tauri-apps/plugin-log";
import { ClipboardList, FolderSearch, Monitor, Moon, RotateCcw, Sun } from "lucide-react";
import { toast } from "sonner";

import { ClipboardStatus } from "@/components/clipboard/clipboard-status";
import { SearchIndexStatus } from "@/components/search/search-status";
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
import { ipc, type AppConfig, type Theme } from "@/lib/ipc";
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

  const cancelIndex = async () => {
    try {
      await ipc.cancelFileIndex();
    } catch (cause) {
      toast.error(`取消失败：${String(cause)}`);
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
          <CardDescription>主题偏好写入配置文件，重启后保持。</CardDescription>
        </CardHeader>
        <CardContent>
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
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>窗口行为</CardTitle>
          <CardDescription>关闭主窗口时是否保留后台托盘图标。</CardDescription>
        </CardHeader>
        <CardContent className="flex items-center justify-between">
          <Label htmlFor="close-to-tray">关闭窗口时最小化到托盘</Label>
          <Switch
            id="close-to-tray"
            checked={config.closeToTray}
            onCheckedChange={(checked) => void apply({ closeToTray: checked })}
          />
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
            不联网。列表、搜索与编辑在「剪贴板」页。
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
}: {
  value: number;
  onCommit: (days: number) => void;
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
      id="clipboard-retention"
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
