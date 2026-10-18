import { useState } from "react";

import {
  Download,
  ExternalLink,
  FolderOpen,
  Pause,
  Play,
  RefreshCw,
  RotateCcw,
  ShieldCheck,
  X,
} from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { useUpdate } from "@/hooks/use-update";
import { formatBytes } from "@/lib/format";
import type { UpdateState } from "@/lib/ipc";

/**
 * 设置页「软件更新」卡片。
 *
 * 链路：Gitee 跳板（latest.json）→ 第三方服务器（安装包 + 清单）→ SHA-256 → NSIS
 * `/S /UPDATE /R` 静默安装并自动重启。所有状态都来自 Rust 的 `update:state` 广播，
 * 这里只负责展示与发起动作。
 */
export function UpdateCard() {
  const { state, check, reinstall, pause, resume, cancel, install, openInstaller, openDownloadUrl } =
    useUpdate();
  const [installOpen, setInstallOpen] = useState(false);
  const [reinstallOpen, setReinstallOpen] = useState(false);
  const [installing, setInstalling] = useState(false);
  const isDev = import.meta.env.DEV;

  if (!state) {
    return (
      <Card>
        <CardHeader>
          <CardTitle>软件更新</CardTitle>
          <CardDescription>正在读取更新状态…</CardDescription>
        </CardHeader>
      </Card>
    );
  }

  const busy = state.status === "checking" || state.status === "downloading";
  const canInstall = state.status === "downloaded" && !isDev;
  const percent = state.progress;
  const showProgress = state.status === "downloading" || state.status === "paused";
  const showSizes = state.totalBytes != null;

  const startInstall = async () => {
    setInstalling(true);
    const ok = await install();
    // 成功时应用会退出；只有安装器起不来（失败）才需要复位界面。
    if (!ok) setInstalling(false);
    setInstallOpen(false);
  };

  return (
    <>
      <Card>
        <CardHeader>
          <CardTitle>软件更新</CardTitle>
          <CardDescription>
            从 Gitee 跳板读取更新清单，安装包由第三方服务器提供；下载后经 SHA-256 校验，
            静默覆盖安装并自动重启。
          </CardDescription>
        </CardHeader>

        <CardContent className="flex flex-col gap-3">
          <div className="flex items-center justify-between gap-3">
            <span className="text-xs text-muted-foreground">当前版本 v{state.currentVersion}</span>
            {state.status === "downloaded" && (
              <span className="inline-flex items-center gap-1 text-xs text-emerald-600 dark:text-emerald-400">
                <ShieldCheck className="size-3.5" />
                已校验，可安装
              </span>
            )}
          </div>

          <p className="text-sm">{statusText(state)}</p>

          {state.notes && (
            <p className="whitespace-pre-wrap text-xs text-muted-foreground">{state.notes}</p>
          )}

          {showProgress && (
            <div className="flex flex-col gap-1.5">
              <div
                className="h-2 w-full overflow-hidden rounded-full bg-muted"
                role="progressbar"
                aria-valuemin={0}
                aria-valuemax={100}
                aria-valuenow={percent ?? undefined}
                aria-label="更新下载进度"
              >
                <div
                  className="h-full rounded-full bg-primary transition-[width] duration-150"
                  style={{ width: `${percent ?? 0}%` }}
                />
              </div>
              <div className="flex items-center justify-between text-xs text-muted-foreground">
                <span>{percent == null ? "总大小未知" : `${percent}%`}</span>
                {showSizes && (
                  <span>
                    {formatBytes(state.downloadedBytes)} / {formatBytes(state.totalBytes ?? 0)}
                  </span>
                )}
              </div>
            </div>
          )}

          {state.status === "error" && state.error && (
            <p className="rounded-md bg-destructive/10 px-3 py-2 text-xs break-all text-destructive">
              {state.error}
            </p>
          )}

          {isDev && (
            <p className="text-xs text-muted-foreground">
              开发模式不自动检查、也不执行安装。联调时设置
              <code className="mx-1 rounded bg-muted px-1 py-0.5">PRISM3_UPDATE_ANCHOR</code>
              指向本地 latest.json 即可检查与下载。
            </p>
          )}

          <div className="flex flex-wrap items-center gap-2">
            <Button
              variant="outline"
              size="sm"
              disabled={busy || state.status === "paused"}
              onClick={() => void check()}
            >
              <RefreshCw
                data-icon="inline-start"
                className={state.status === "checking" ? "animate-spin" : undefined}
              />
              检查更新
            </Button>

            {state.status === "downloading" && (
              <Button variant="outline" size="sm" onClick={() => void pause()}>
                <Pause data-icon="inline-start" />
                暂停
              </Button>
            )}

            {state.status === "paused" && (
              <Button variant="outline" size="sm" onClick={() => void resume()}>
                <Play data-icon="inline-start" />
                继续
              </Button>
            )}

            {(state.status === "checking" ||
              state.status === "downloading" ||
              state.status === "paused") && (
              <Button variant="ghost" size="sm" onClick={() => void cancel()}>
                <X data-icon="inline-start" />
                取消
              </Button>
            )}

            {canInstall && (
              <Button size="sm" onClick={() => setInstallOpen(true)} disabled={installing}>
                <Download data-icon="inline-start" />
                {installing ? "正在重启…" : "立即重启并安装"}
              </Button>
            )}

            {state.status === "error" && state.installerPath && (
              <Button variant="outline" size="sm" onClick={() => void openInstaller()}>
                <FolderOpen data-icon="inline-start" />
                打开安装包
              </Button>
            )}

            {state.status === "error" && state.downloadUrl && (
              <Button variant="outline" size="sm" onClick={() => void openDownloadUrl()}>
                <ExternalLink data-icon="inline-start" />
                下载安装包
              </Button>
            )}
          </div>
        </CardContent>

        {!isDev && (
          <CardFooter className="justify-between gap-3">
            <span className="text-xs text-muted-foreground">
              不比较版本号，直接重新下载并安装最新版（用于验证安装链路）。
            </span>
            <Button
              variant="ghost"
              size="sm"
              disabled={busy || state.status === "paused" || state.status === "downloaded"}
              onClick={() => setReinstallOpen(true)}
            >
              <RotateCcw data-icon="inline-start" />
              重新安装
            </Button>
          </CardFooter>
        )}
      </Card>

      <Dialog open={installOpen} onOpenChange={setInstallOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>安装 v{state.version ?? "新版本"} 并重启？</DialogTitle>
            <DialogDescription>
              应用会立即退出，安装器在后台静默覆盖安装，完成后自动启动新版本。
              请先保存正在编辑的内容。
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setInstallOpen(false)} disabled={installing}>
              取消
            </Button>
            <Button onClick={() => void startInstall()} disabled={installing}>
              {installing ? "正在重启…" : "安装并重启"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={reinstallOpen} onOpenChange={setReinstallOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>重新安装最新版本？</DialogTitle>
            <DialogDescription>
              不比较版本号，直接重新下载并安装最新版安装包；当前版本会被覆盖。
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setReinstallOpen(false)}>
              取消
            </Button>
            <Button
              onClick={() => {
                setReinstallOpen(false);
                void reinstall();
              }}
            >
              重新安装
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}

function statusText(state: UpdateState): string {
  switch (state.status) {
    case "idle":
      return state.message ?? "尚未检查更新";
    case "checking":
      return "正在检查更新…";
    case "upToDate":
      return "已是最新版本";
    case "available":
      return `发现新版本 v${state.version ?? ""}，准备下载…`;
    case "downloading":
      return `正在下载 v${state.version ?? ""}…`;
    case "paused":
      return `下载已暂停${state.progress == null ? "" : `（${state.progress}%）`}`;
    case "downloaded":
      return `v${state.version ?? ""} 已下载并通过 SHA-256 校验，重启后生效`;
    case "error":
      return "更新失败";
    default:
      return "未知状态";
  }
}
