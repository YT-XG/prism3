import { useEffect } from "react";

import { AppShell } from "@/components/app/app-shell";
import { ThemeProvider } from "@/components/theme-provider";
import { Toaster } from "@/components/ui/sonner";
import { useConfig } from "@/hooks/use-config";
import { useUpdateNotifications } from "@/hooks/use-update";

function App() {
  useConfig();
  // 启动 3s 后的静默检查发生在设置页还没打开的时候，所以更新提示挂在全局。
  useUpdateNotifications();

  // 首帧提交之后淡出 index.html 里的启动占位（它保证窗口一出现就有内容，
  // 而不是在 dev 模式下白 3 秒）。放在 App 里而不是 main.tsx：这里一定已经提交。
  useEffect(() => {
    const splash = document.getElementById("boot-splash");
    if (!splash) return;
    const frame = requestAnimationFrame(() => {
      splash.classList.add("is-hidden");
      window.setTimeout(() => splash.remove(), 220);
    });
    return () => cancelAnimationFrame(frame);
  }, []);

  return (
    <ThemeProvider>
      <AppShell />
      <Toaster />
    </ThemeProvider>
  );
}

export default App;
