import { useEffect, useState } from "react";

import { getVersion } from "@tauri-apps/api/app";
import { error as logError } from "@tauri-apps/plugin-log";

/**
 * 应用版本号（`tauri.conf.json` 的 `version`）。
 *
 * 走运行时 `getVersion()` 而不是打包时注入常量：发版仍然只改那三处版本号
 * （`package.json` / `Cargo.toml` / `tauri.conf.json`，见 README），前端不用跟着改，
 * 也不会出现「界面上写着旧版本」的漂移。
 *
 * 读回来之前返回空串，调用方按「没有就不显示」处理，避免闪一下 `vundefined`。
 */
export function useAppVersion(): string {
  const [version, setVersion] = useState("");

  useEffect(() => {
    let active = true;
    void getVersion()
      .then((next) => {
        if (active) setVersion(next);
      })
      .catch((cause) => {
        // 只影响这一处文案，记一条日志即可。
        void logError(`读取应用版本失败：${String(cause)}`);
      });
    return () => {
      active = false;
    };
  }, []);

  return version;
}