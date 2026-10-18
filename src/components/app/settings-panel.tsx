import { info } from "@tauri-apps/plugin-log";
import { FolderSearch, Monitor, Moon, RotateCcw, Sun } from "lucide-react";
import { toast } from "sonner";

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
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useFileIndexState } from "@/hooks/use-file-index-state";
import { ipc, type AppConfig, type Theme } from "@/lib/ipc";
import { useAppStore } from "@/stores/app-store";

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
          <CardTitle>重置</CardTitle>
          <CardDescription>把主题与窗口行为恢复为默认值。</CardDescription>
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
