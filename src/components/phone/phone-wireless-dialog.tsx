import { useEffect, useState } from "react";

import { toast } from "sonner";
import { Cable, Loader2, RefreshCw, Search, Unplug, Wifi } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { usePhoneWireless } from "@/hooks/use-phone";

interface PhoneWirelessDialogProps {
  open: boolean;
  onClose: () => void;
  /** 连上 / 断开之后通知外面重新探测设备（`phone_serial` 已经指向新地址）。 */
  onChanged: () => void;
}

/**
 * 「无线调试」对话框：把 adb 从数据线切到 Wi-Fi。
 *
 * 为什么值得做：**一直插着数据线，手机就会一直充电**。切到无线之后拔掉线，
 * 手机既不充电、镜像与触控又照旧——这是唯一「软件层面真能解决」的办法
 * （充电由 PMIC 硬件完成，未 root 的 Android 改不了，见 `phone.rs` 的说明）。
 *
 * 两条路径：
 * 1. **一键切**（大多数机器）：USB 还在 → `adb tcpip 5555` → `adb connect 手机IP:5555`；
 * 2. **配对**（Android 11+ 的「无线调试」页面）：手机会给随机端口 + 6 位码，
 *    走 `adb pair`，之后用同一地址连接。`adb tcpip` 被系统挡掉时才需要它。
 */
export function PhoneWirelessDialog({ open, onClose, onChanged }: PhoneWirelessDialogProps) {
  const { info, busy, refresh, enable, discover, connect, disconnect, pair } =
    usePhoneWireless();
  const [address, setAddress] = useState("");
  const [code, setCode] = useState("");
  const [showPair, setShowPair] = useState(false);
  /** 每次打开只自动找一次，避免「关不掉的反复扫描」。 */
  const [autoTried, setAutoTried] = useState(false);

  // 打开时拉一次最新状态；后端给了建议地址就把它填进输入框（用户可改）。
  useEffect(() => {
    if (!open) return;
    void refresh().then(() => undefined);
  }, [open, refresh]);

  // 自己找一次：没连着就试「记住的地址 → mDNS → 邻居表」，成功与否都不打扰——
  // 用户真点「自动查找」时才会有 toast。
  useEffect(() => {
    if (!open) {
      setAutoTried(false);
      return;
    }
    if (autoTried || info === null || info.connected) {
      if (info?.connected) setAutoTried(true);
      return;
    }
    setAutoTried(true);
    void discover().catch(() => undefined);
  }, [open, info, autoTried, discover]);

  useEffect(() => {
    if (open && info?.address && address === "") setAddress(info.address);
    // 只在「刚拿到建议地址」时预填一次，之后不再覆盖用户的输入。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, info?.address]);

  const pending = busy !== null;

  const run = async (label: string, action: () => Promise<void>) => {
    try {
      await action();
      onChanged();
      toast.success(label);
    } catch (cause) {
      toast.error(String(cause));
    }
  };

  return (
    <Dialog open={open} onOpenChange={(next) => (!next ? onClose() : undefined)}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Wifi className="size-4" />
            无线调试（拔掉数据线也不再充电）
          </DialogTitle>
          <DialogDescription>
            手机一直插着线就会一直充电。切到 Wi-Fi 之后拔线，镜像和触控照旧。
            <br />
            要求：手机与电脑在同一个局域网——电脑开热点、手机连它是最好用的做法
            （Windows 的移动热点会给电脑 192.168.137.1，手机拿到 192.168.137.x）。
          </DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-3 text-sm">
          {/* 现状：只管事实，不解释。 */}
          <div className="flex flex-wrap items-center gap-x-4 gap-y-1 rounded-lg border bg-muted/40 px-3 py-2 text-xs">
            <span className="flex items-center gap-1.5">
              <Cable className="size-3.5" />
              USB：{info?.usbSerial ?? "未连接"}
            </span>
            <span>手机 IP：{info?.ip ?? "未知"}</span>
            <span>adbd 端口：{info?.tcpPort ?? "—"}</span>
            <span className={info?.connected ? "text-emerald-600" : "text-muted-foreground"}>
              {info?.connected ? `已连上 ${info.address ?? ""}` : "未连上 Wi-Fi"}
            </span>
          </div>

          {/* 主操作：能一键切就一键切，不能就说明为什么。 */}
          {info?.connected ? (
            <Button
              variant="outline"
              disabled={pending}
              onClick={() =>
                // 断开**不清空**地址输入框：紧接着最可能做的就是「再连一次」。
                void run("已断开无线连接", async () => {
                  await disconnect(info.address ?? address);
                })
              }
            >
              <Unplug />
              断开无线（这条不影响 USB）
            </Button>
          ) : (
            <div className="flex flex-wrap items-center gap-2">
              <Button
                disabled={pending || !info?.usbSerial || !info?.ip}
                onClick={() => void run("已切到无线调试，可以拔线了", async () => {
                  await enable();
                })}
              >
                {busy === "enable" ? <Loader2 className="animate-spin" /> : <Wifi />}
                一键切到无线调试
              </Button>
              {/* 不用手填 IP：自动按「记住的地址 → mDNS → 邻居表」找。 */}
              <Button
                variant="secondary"
                disabled={pending}
                onClick={() =>
                  void run("已通过 Wi-Fi 连上手机", async () => {
                    await discover();
                  })
                }
              >
                {busy === "discover" ? <Loader2 className="animate-spin" /> : <Search />}
                自动查找手机
              </Button>
            </div>
          )}

          {!info?.usbSerial && !info?.connected ? (
            <p className="text-xs text-muted-foreground">
              现在没有 USB 连着，没法替手机切 `tcpip`。先插一次线，或直接在下面填地址连接。
            </p>
          ) : null}
          {info?.usbSerial && !info?.ip ? (
            <p className="text-xs text-muted-foreground">
              手机没有局域网 IP：它现在走的是移动数据。让手机连上电脑热点（Windows 设置 → 网络和
              Internet → 移动热点），拿到 192.168.137.x 之后再点上面的按钮。
            </p>
          ) : null}

          {/* 兜底：地址可以手填，IP 变了、换网络都靠它。 */}
          <div className="flex flex-col gap-1.5">
            <label className="text-xs text-muted-foreground" htmlFor="phone-wireless-address">
              手动连接地址（`手机IP:端口`）
            </label>
            <div className="flex items-center gap-2">
              <Input
                id="phone-wireless-address"
                value={address}
                placeholder="192.168.137.232:5555"
                onChange={(event) => setAddress(event.target.value)}
                className="h-8"
              />
              <Button
                size="sm"
                variant="secondary"
                disabled={pending || address.trim() === ""}
                onClick={() =>
                  void run(`已连上 ${address.trim()}`, async () => {
                    await connect(address.trim());
                  })
                }
              >
                {busy === "connect" ? <Loader2 className="animate-spin" /> : null}
                连接
              </Button>
              <Button
                size="icon-sm"
                variant="ghost"
                title="重新探测"
                disabled={pending}
                onClick={() => void refresh()}
              >
                <RefreshCw />
              </Button>
            </div>
          </div>

          {/* 配对：Android 11+「无线调试」页面给随机端口 + 6 位码。 */}
          <div className="flex flex-col gap-1.5">
            <button
              type="button"
              className="w-fit text-xs text-muted-foreground underline-offset-2 hover:underline"
              onClick={() => setShowPair((value) => !value)}
            >
              {showPair ? "收起配对" : "手机上有「无线调试」页面？用配对码连"}
            </button>
            {showPair ? (
              <div className="flex items-center gap-2">
                <Input
                  value={address}
                  placeholder="地址（与上面同一个）"
                  onChange={(event) => setAddress(event.target.value)}
                  className="h-8"
                />
                <Input
                  value={code}
                  placeholder="6 位配对码"
                  inputMode="numeric"
                  onChange={(event) => setCode(event.target.value)}
                  className="h-8 w-28"
                />
                <Button
                  size="sm"
                  variant="secondary"
                  disabled={pending || address.trim() === "" || code.trim() === ""}
                  onClick={() =>
                    void run("配对成功，接着点「连接」", async () => {
                      await pair(address.trim(), code.trim());
                      setCode("");
                    })
                  }
                >
                  {busy === "pair" ? <Loader2 className="animate-spin" /> : null}
                  配对
                </Button>
              </div>
            ) : null}
          </div>
        </div>

        <DialogFooter className="text-xs text-muted-foreground sm:justify-start">
          连上之后直接拔数据线：手机会停止充电，adb 走 Wi-Fi。手机深度睡眠时可能掉线，回来点一下「连接」即可。
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
