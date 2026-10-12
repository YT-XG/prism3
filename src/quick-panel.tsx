import { StrictMode } from "react";
import ReactDOM from "react-dom/client";

import { getCurrentWindow } from "@tauri-apps/api/window";
import { attachConsole } from "@tauri-apps/plugin-log";

import { QuickPanel } from "./components/quick-panel/quick-panel";
import { QuickPanelBody } from "./components/quick-panel/quick-panel-body";
import "./quick-panel.css";

// 开发时把 Rust 侧日志镜像到 devtools 控制台。
if (import.meta.env.DEV) {
  void attachConsole();
}

// 灰条与面板是两个窗口、共用同一个 HTML 入口，按窗口 label 分派：
// `quick-panel` = 灰条（常驻、独占交互与状态），`quick-panel-body` = 面板（按需显示）。
const isBodyWindow = getCurrentWindow().label === "quick-panel-body";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <StrictMode>{isBodyWindow ? <QuickPanelBody /> : <QuickPanel />}</StrictMode>,
);
