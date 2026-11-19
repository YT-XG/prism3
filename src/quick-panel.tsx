import { StrictMode } from "react";
import ReactDOM from "react-dom/client";

import { getCurrentWindow } from "@tauri-apps/api/window";
import { attachConsole, error as logError } from "@tauri-apps/plugin-log";

import { HubPanel } from "./components/hub/hub-panel";
import { QuickPanel } from "./components/quick-panel/quick-panel";
import { QuickPanelBody } from "./components/quick-panel/quick-panel-body";
import "./quick-panel.css";

// 开发时把 Rust 侧日志镜像到 devtools 控制台。
if (import.meta.env.DEV) {
  void attachConsole();
}

/**
 * 三个窗口共用同一个 HTML 入口，按窗口 label 分派：
 * `quick-panel` = 灰条（常驻、独占交互与状态），`quick-panel-body` = 面板（按需显示），
 * `hub` = 组合窗口（透明大窗口里并排卡片，全局快捷键唤起）。
 */
const label = getCurrentWindow().label;

/**
 * 任何一个窗口在渲染期抛错都会让整个窗口变成透明空白。
 * 把未捕获异常写进 Rust 日志（release 也保留），否则「面板/组合窗口打不开」
 * 只能看到窗口出现又消失，日志里什么都没有。
 */
function reportFrontendError(reason: unknown) {
  const detail = reason instanceof Error ? `${reason.message}\n${reason.stack ?? ""}` : String(reason);
  void logError(`前端未捕获异常（${label}）：${detail}`);
}
window.addEventListener("error", (event) => {
  reportFrontendError(event.error ?? event.message);
});
window.addEventListener("unhandledrejection", (event) => {
  reportFrontendError(event.reason);
});

function Root() {
  if (label === "hub") return <HubPanel />;
  if (label === "quick-panel-body") return <QuickPanelBody />;
  return <QuickPanel />;
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <StrictMode>
    <Root />
  </StrictMode>,
);
