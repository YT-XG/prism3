import { StrictMode } from "react";
import ReactDOM from "react-dom/client";

import { attachConsole } from "@tauri-apps/plugin-log";

import App from "./App";
import "./index.css";

// 开发时把 Rust 侧日志镜像到 devtools 控制台。
if (import.meta.env.DEV) {
  void attachConsole();
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
