import { useCallback, useEffect, useRef, useState } from "react";

import { open } from "@tauri-apps/plugin-dialog";
import {
  CornerUpLeft,
  Download,
  FolderOpen,
  House,
  Layers,
  Lock,
  Pause,
  Play,
  RefreshCw,
  Smartphone,
  Sun,
  Volume1,
  Volume2,
  type LucideIcon,
} from "lucide-react";
import { toast } from "sonner";

import { PageHeader } from "@/components/app/page-header";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { usePhone, usePhoneMirror } from "@/hooks/use-phone";
import { ipc, type PhoneKey } from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { useAppStore } from "@/stores/app-store";

/** 点按与滑动的分界（设备像素）：手指抖一下不算滑动。 */
const TAP_THRESHOLD = 14;

/** 镜像区的辅助按键（锁屏单独做主按钮，它是这一页的主诉求）。 */
const KEY_ACTIONS: { key: PhoneKey; label: string; icon: LucideIcon }[] = [
  { key: "wake", label: "唤醒", icon: Sun },
  { key: "back", label: "返回", icon: CornerUpLeft },
  { key: "home", label: "主页", icon: House },
  { key: "recents", label: "最近任务", icon: Layers },
  { key: "volumeDown", label: "音量-", icon: Volume1 },
  { key: "volumeUp", label: "音量+", icon: Volume2 },
];

interface Point {
  x: number;
  y: number;
}

/**
 * 「手机」页：用数据线连着的那台 Android 手机的**镜像 + 触控 + 锁屏**。
 *
 * 三件事都走 adb（见 `src-tauri/src/phone.rs`）：截屏轮询当画面，
 * 画布上的点按/拖动换算成设备坐标后注入 `input tap` / `input swipe`，
 * 锁屏走 `KEYCODE_SLEEP`（不生效才退电源键，且读不到屏幕状态时不乱按）。
 *
 * 页面本身的四种「不通」状态都要说清楚原因，不能只留一片空白：
 * 没有 adb → 可一键下载；没有设备 → 查线/查调试开关；未授权 → 去手机上点允许。
 */
export function PhonePanel() {
  const { status, probing, installing, error, probe, install, select } = usePhone();
  const saveConfig = useAppStore((state) => state.saveConfig);

  const [playing, setPlaying] = useState(true);
  const [busy, setBusy] = useState<string | null>(null);
  const [box, setBox] = useState({ width: 0, height: 0 });
  const [trail, setTrail] = useState<{ from: Point; to: Point } | null>(null);

  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const stageRef = useRef<HTMLDivElement | null>(null);
  const dragRef = useRef<{ start: Point; startedAt: number } | null>(null);

  const serial = status?.selected ?? null;
  const device = status?.devices.find((item) => item.serial === serial) ?? null;
  const ready = Boolean(serial && device?.authorized);
  const mirror = usePhoneMirror({
    canvasRef,
    serial: ready ? serial : null,
    enabled: playing,
  });

  // 舞台尺寸：镜像按「等比缩放、不超出舞台」显示，坐标换算也依赖这个比例。
  useEffect(() => {
    const stage = stageRef.current;
    if (!stage) return;
    const observer = new ResizeObserver((entries) => {
      const rect = entries[0]?.contentRect;
      if (rect) setBox({ width: rect.width, height: rect.height });
    });
    observer.observe(stage);
    return () => observer.disconnect();
  }, [ready]);

  const display =
    mirror.size && box.width > 0 && box.height > 0
      ? (() => {
          const scale = Math.min(
            box.width / mirror.size.width,
            box.height / mirror.size.height,
            1,
          );
          return {
            width: Math.round(mirror.size.width * scale),
            height: Math.round(mirror.size.height * scale),
          };
        })()
      : null;

  const runAction = useCallback(
    async (action: () => Promise<unknown>) => {
      setBusy("busy");
      try {
        await action();
      } catch (cause) {
        // 后端错误已经是「点击失败：…」这种完整句子，不再套一层前缀。
        toast.error(String(cause));
      } finally {
        setBusy(null);
      }
    },
    [],
  );

  /** 屏幕坐标 → 设备像素。必须以 canvas 当前的实际显示尺寸为准（窗口可缩放）。 */
  const devicePoint = (event: React.PointerEvent<HTMLCanvasElement>): Point | null => {
    const canvas = canvasRef.current;
    if (!canvas || !mirror.size) return null;
    const rect = canvas.getBoundingClientRect();
    if (rect.width === 0 || rect.height === 0) return null;
    const x = Math.round(((event.clientX - rect.left) / rect.width) * mirror.size.width);
    const y = Math.round(((event.clientY - rect.top) / rect.height) * mirror.size.height);
    return {
      x: Math.min(Math.max(x, 0), mirror.size.width - 1),
      y: Math.min(Math.max(y, 0), mirror.size.height - 1),
    };
  };

  const handlePointerDown = (event: React.PointerEvent<HTMLCanvasElement>) => {
    if (!serial) return;
    const point = devicePoint(event);
    if (!point) return;
    event.currentTarget.setPointerCapture(event.pointerId);
    dragRef.current = { start: point, startedAt: performance.now() };
    setTrail(null);
  };

  const handlePointerMove = (event: React.PointerEvent<HTMLCanvasElement>) => {
    const drag = dragRef.current;
    if (!drag) return;
    const point = devicePoint(event);
    if (point) setTrail({ from: drag.start, to: point });
  };

  const handlePointerUp = (event: React.PointerEvent<HTMLCanvasElement>) => {
    const drag = dragRef.current;
    dragRef.current = null;
    setTrail(null);
    if (!drag || !serial) return;

    const point = devicePoint(event);
    if (!point) return;
    const distance = Math.hypot(point.x - drag.start.x, point.y - drag.start.y);
    const duration = Math.min(
      800,
      Math.max(60, Math.round(performance.now() - drag.startedAt)),
    );

    if (distance < TAP_THRESHOLD) {
      void runAction(() => ipc.phoneTap(serial, point.x, point.y));
    } else {
      void runAction(() =>
        ipc.phoneSwipe(serial, drag.start.x, drag.start.y, point.x, point.y, duration),
      );
    }
  };

  const handlePointerCancel = () => {
    dragRef.current = null;
    setTrail(null);
  };

  const downloadAdb = async () => {
    const ok = await install();
    if (ok) toast.success("platform-tools 已就绪");
    else toast.error("下载 platform-tools 失败，详情见页面提示");
  };

  /** 手动指定系统里已有的 adb.exe（下载失败 / 公司网络拦截时的退路）。 */
  const pickAdb = async () => {
    const picked = await open({
      multiple: false,
      directory: false,
      title: "选择 adb.exe",
      filters: [{ name: "adb", extensions: ["exe"] }],
    });
    if (typeof picked !== "string") return;
    try {
      await saveConfig({ phoneAdbPath: picked });
      await probe();
    } catch (cause) {
      toast.error(`保存 adb 路径失败：${String(cause)}`);
    }
  };

  const lock = () => {
    if (!serial) return;
    void runAction(async () => {
      const message = await ipc.phoneLock(serial);
      toast.success(message);
    });
  };

  const deviceLabel = device ? device.model || device.serial : "未连接";

  return (
    <div className="flex h-full min-h-0 flex-col gap-3">
      <PageHeader
        title="手机"
        badge={
          <div className="flex flex-wrap items-center gap-1.5">
            <Badge variant={ready ? "secondary" : "outline"} className="max-w-[180px] truncate">
              {deviceLabel}
            </Badge>
            {ready && mirror.size ? (
              <Badge variant="outline">
                {mirror.size.width}×{mirror.size.height}
              </Badge>
            ) : null}
            {ready ? (
              <Badge variant="outline" className="tabular-nums">
                {playing ? `${mirror.fps || "—"} fps` : "已暂停"}
              </Badge>
            ) : null}
          </div>
        }
        actions={
          <div className="flex items-center gap-1">
            <Button
              size="icon-sm"
              variant="ghost"
              onClick={() => setPlaying((previous) => !previous)}
              disabled={!ready}
              aria-label={playing ? "暂停画面" : "继续画面"}
              title={playing ? "暂停画面（省手机电）" : "继续画面"}
            >
              {playing ? <Pause /> : <Play />}
            </Button>
            <Button
              size="icon-sm"
              variant="ghost"
              onClick={() => void probe()}
              disabled={probing}
              aria-label="重新检测"
              title="重新检测 adb 与设备"
            >
              <RefreshCw className={cn(probing && "animate-spin")} />
            </Button>
          </div>
        }
      />

      {!status ? (
        <Card>
          <CardHeader>
            <CardTitle>正在探测 adb 与设备…</CardTitle>
            <CardDescription>首次会顺手拉起 adb server，可能要多等一两秒。</CardDescription>
          </CardHeader>
        </Card>
      ) : !status.adbFound ? (
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <Smartphone className="size-4" />
              还没有 adb
            </CardTitle>
            <CardDescription>
              手机控制靠 adb 截屏与注入触摸。可以在这里下载一份 platform-tools（约 8 MB），
              它只解压到应用数据目录，不会写系统 PATH；也可以指定你已经装好的 adb.exe。
            </CardDescription>
          </CardHeader>
          <CardContent className="flex flex-wrap items-center gap-2">
            <Button onClick={() => void downloadAdb()} disabled={installing}>
              <Download />
              {installing ? "正在下载…" : "下载 platform-tools"}
            </Button>
            <Button variant="outline" onClick={() => void pickAdb()} disabled={installing}>
              <FolderOpen />
              手动指定 adb.exe
            </Button>
            {error ? <p className="w-full text-xs text-destructive">{error}</p> : null}
          </CardContent>
        </Card>
      ) : !ready ? (
        <Card>
          <CardHeader>
            <CardTitle>
              {status.devices.length === 0 ? "没检测到手机" : "等待手机授权"}
            </CardTitle>
            <CardDescription>
              {status.devices.length === 0
                ? "用数据线连上电脑、打开「开发者选项 → USB 调试」，连接方式选「传输文件（MTP）」——仅充电不会出现 ADB 接口。"
                : `手机上会弹出「允许 USB 调试吗？」，勾选「一律允许」再点允许。没有弹窗就拔插一次数据线，或到开发者选项里点「撤销 USB 调试授权」再来一次。（当前状态：${
                    device?.state ?? "未知"
                  }）`}
            </CardDescription>
          </CardHeader>
          <CardContent className="flex flex-col gap-2">
            <div className="flex flex-wrap items-center gap-2">
              <Button variant="outline" onClick={() => void probe()} disabled={probing}>
                <RefreshCw className={cn(probing && "animate-spin")} />
                重新检测
              </Button>
              {status.devices.length > 1 ? (
                <div className="flex flex-wrap items-center gap-1.5">
                  {status.devices.map((item) => (
                    <Button
                      key={item.serial}
                      size="sm"
                      variant={item.serial === status.selected ? "secondary" : "ghost"}
                      onClick={() => void select(item.serial)}
                    >
                      {item.model || item.serial}
                    </Button>
                  ))}
                </div>
              ) : null}
            </div>
            {status.error ? (
              <p className="text-xs text-destructive">{status.error}</p>
            ) : null}
          </CardContent>
        </Card>
      ) : (
        <div className="flex min-h-0 flex-1 flex-col gap-2">
          <div
            ref={stageRef}
            className="relative flex min-h-0 flex-1 items-center justify-center overflow-hidden rounded-xl border border-border/70 bg-[#0b0d12] p-2"
          >
            {display && mirror.size ? (
              <div
                className="relative"
                style={{ width: display.width, height: display.height }}
              >
                <canvas
                  ref={canvasRef}
                  width={mirror.size.width}
                  height={mirror.size.height}
                  className="block h-full w-full cursor-crosshair rounded-lg shadow-2xl"
                  style={{ touchAction: "none" }}
                  onPointerDown={handlePointerDown}
                  onPointerMove={handlePointerMove}
                  onPointerUp={handlePointerUp}
                  onPointerCancel={handlePointerCancel}
                />
                {trail ? (
                  // viewBox 用设备像素：SVG 跟着外层一起缩放，轨迹坐标不用再乘比例。
                  <svg
                    className="pointer-events-none absolute inset-0"
                    viewBox={`0 0 ${mirror.size.width} ${mirror.size.height}`}
                  >
                    <line
                      x1={trail.from.x}
                      y1={trail.from.y}
                      x2={trail.to.x}
                      y2={trail.to.y}
                      stroke="white"
                      strokeOpacity={0.7}
                      strokeWidth={Math.max(2, mirror.size.width / 140)}
                      strokeLinecap="round"
                    />
                    <circle
                      cx={trail.from.x}
                      cy={trail.from.y}
                      r={Math.max(4, mirror.size.width / 90)}
                      fill="white"
                      fillOpacity={0.55}
                    />
                  </svg>
                ) : null}
              </div>
            ) : (
              <p className="text-sm text-white/60">正在获取第一帧…</p>
            )}
          </div>

          <p className="text-center text-[11px] text-muted-foreground">
            画面上单击 = 点按，按住拖动 = 滑动
            {mirror.size ? `（坐标按 ${mirror.size.width}×${mirror.size.height} 换算）` : ""}
          </p>

          <div className="flex flex-wrap items-center justify-center gap-1.5">
            <Button onClick={lock} disabled={busy !== null}>
              <Lock />
              锁屏
            </Button>
            {KEY_ACTIONS.map(({ key, label, icon: Icon }) => (
              <Button
                key={key}
                size="sm"
                variant="outline"
                disabled={busy !== null}
                onClick={() => {
                  if (!serial) return;
                  void runAction(() => ipc.phoneKey(serial, key));
                }}
                title={label}
                aria-label={label}
              >
                <Icon />
                {label}
              </Button>
            ))}
          </div>

          {mirror.error ? (
            <p className="text-center text-xs text-destructive">{mirror.error}</p>
          ) : null}

          {status.devices.length > 1 ? (
            <div className="flex flex-wrap items-center justify-center gap-1.5">
              {status.devices.map((item) => (
                <Button
                  key={item.serial}
                  size="xs"
                  variant={item.serial === status.selected ? "secondary" : "ghost"}
                  onClick={() => void select(item.serial)}
                >
                  {item.model || item.serial}
                </Button>
              ))}
            </div>
          ) : null}
        </div>
      )}
    </div>
  );
}
