import { Badge } from "@/components/ui/badge";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { ScrollArea } from "@/components/ui/scroll-area";
import { MonitorPanel } from "@/components/monitor/monitor-panel";
import { cn } from "@/lib/utils";
import { useAppStore } from "@/stores/app-store";

import { AppSidebar } from "./sidebar-nav";
import { SettingsPanel } from "./settings-panel";
import { TitleBar } from "./title-bar";

/**
 * 一体式外壳：标题栏与左侧导航共用 `bg-sidebar` 连成 L 形，
 * 内容区是独立的 `bg-background` 面板，左上角圆角露出底层底色。
 */
export function AppShell() {
  const activeView = useAppStore((state) => state.activeView);

  // 数据型页面（监控）充分利用窗口宽度，表单型页面保持窄栏更易读。
  const contentWidth = {
    home: "max-w-3xl",
    settings: "max-w-3xl",
    monitor: "max-w-6xl",
  }[activeView];

  return (
    <div className="flex h-screen w-screen flex-col overflow-hidden bg-sidebar text-sidebar-foreground">
      <TitleBar />

      <div className="flex min-h-0 flex-1">
        <AppSidebar />

        <main className="min-w-0 flex-1 overflow-hidden rounded-tl-xl border-t border-l border-border/60 bg-background text-foreground">
          <ScrollArea className="h-full">
            <div className={cn("mx-auto w-full p-6", contentWidth)}>
              {activeView === "home" && <HomeView />}
              {activeView === "settings" && <SettingsPanel />}
              {activeView === "monitor" && <MonitorPanel />}
            </div>
          </ScrollArea>
        </main>
      </div>
    </div>
  );
}

function HomeView() {
  const config = useAppStore((state) => state.config);

  return (
    <div className="flex flex-col gap-4">
      <div className="flex items-center gap-2">
        <h1 className="font-heading text-lg font-medium">Prism3</h1>
        <Badge variant="secondary">脚手架就绪</Badge>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>已接通的基础设施</CardTitle>
          <CardDescription>空壳可直接开始写业务功能。</CardDescription>
        </CardHeader>
        <CardContent>
          <ul className="list-inside list-disc space-y-1 text-sm text-muted-foreground">
            <li>配置持久化：Rust 侧 AppConfig + 原子写盘</li>
            <li>日志：tauri-plugin-log 单管道，前后端写入同一文件</li>
            <li>统一错误处理：thiserror + IPC 可序列化</li>
            <li>自定义标题栏与系统托盘</li>
            <li>窗口状态记忆与单实例保护</li>
          </ul>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>当前配置</CardTitle>
          <CardDescription>来自 Rust 后端的 get_config 命令。</CardDescription>
        </CardHeader>
        <CardContent>
          <pre className="overflow-x-auto rounded-lg bg-muted p-3 text-xs">
            {JSON.stringify(config, null, 2)}
          </pre>
        </CardContent>
      </Card>
    </div>
  );
}
