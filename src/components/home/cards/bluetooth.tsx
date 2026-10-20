import { BatteryCharging, BluetoothOff, Headphones } from "lucide-react";

import { HomeCardHint, HomeCardLink } from "@/components/home/home-card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { useEarbuds } from "@/hooks/use-earbuds";
import { displayDeviceName, oneBudUnavailable } from "@/lib/ble-format";
import type { NoiseMode } from "@/lib/ipc";
import { useAppStore } from "@/stores/app-store";

const PHASE_LABEL: Record<string, string> = {
  idle: "未连接",
  scanning: "扫描中…",
  connecting: "连接中…",
  ready: "已连接",
  error: "连接异常",
};

const NOISE_LABEL: Record<NoiseMode, string> = {
  anc: "降噪",
  normal: "正常",
  wind: "抗风噪",
  transparency: "通透",
};

/** 首页「蓝牙耳机」卡片：只读状态 + 电量 / 降噪标签，操作统一去完整页。 */
export function BluetoothCard() {
  const setActiveView = useAppStore((state) => state.setActiveView);
  const { state, readBattery } = useEarbuds();

  if (!state) {
    return <HomeCardHint>正在读取耳机状态…</HomeCardHint>;
  }

  const ready = state.phase === "ready";
  const phaseLabel = PHASE_LABEL[state.phase] ?? state.phase;
  const name = state.name || state.address
    ? displayDeviceName(state.name, state.address ?? "")
    : "";
  const battery = state.battery;
  const budHint = oneBudUnavailable(battery);

  const tags: string[] = [];
  if (state.noiseMode) {
    tags.push(NOISE_LABEL[state.noiseMode]);
    if (state.adaptiveAnc) tags.push("自适应");
  }
  if (state.gameMode) tags.push("游戏模式");
  if (state.dualDevice) tags.push("双设备");

  return (
    <div className="flex h-full min-h-0 flex-col gap-2.5">
      <div className="flex items-start justify-between gap-2">
        <div className="flex min-w-0 items-center gap-1.5">
          {ready ? (
            <Headphones className="size-4 shrink-0 text-brand" aria-hidden="true" />
          ) : (
            <BluetoothOff className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
          )}
          <div className="min-w-0">
            <p className="truncate text-sm font-medium">{name || "未连接耳机"}</p>
            <p className="truncate text-[0.65rem] text-muted-foreground">
              {ready
                ? state.address ?? "已连接"
                : `${phaseLabel}${state.watching ? " · 正在监听" : ""}`}
            </p>
          </div>
        </div>
        <Badge variant={ready ? "secondary" : "outline"} className="shrink-0">
          {phaseLabel}
        </Badge>
      </div>

      {ready && battery && (
        <div className="grid grid-cols-3 gap-1.5">
          <BatteryCell label="左耳" value={battery.left} charging={battery.leftCharging} />
          <BatteryCell label="右耳" value={battery.right} charging={battery.rightCharging} />
          <BatteryCell label="充电仓" value={battery.case} />
        </div>
      )}

      {ready && !battery && (
        <Button
          variant="outline"
          size="xs"
          className="self-start"
          onClick={() => void readBattery()}
        >
          <BatteryCharging data-icon="inline-start" />
          读取电量
        </Button>
      )}

      {ready && tags.length > 0 && (
        <div className="flex flex-wrap gap-1">
          {tags.map((tag) => (
            <Badge key={tag} variant="secondary">
              {tag}
            </Badge>
          ))}
        </div>
      )}

      {ready && budHint && (
        <p className="text-[0.65rem] text-muted-foreground">
          有一只耳机没有上报电量；只有一只在线时，降噪梯度可能切换不动。
        </p>
      )}

      {state.error && (
        <p className="text-[0.65rem] leading-relaxed text-destructive">{state.error}</p>
      )}

      {!ready && state.activeAddress && (
        <HomeCardHint>
          自动连接目标：{displayDeviceName(state.name, state.activeAddress)}
          {state.autoConnect ? "，取出耳机后会自动连接。" : "（自动连接未开启）。"}
        </HomeCardHint>
      )}

      <div className="mt-auto flex items-center justify-between gap-2 border-t border-border/50 pt-2">
        <span className="truncate text-[0.65rem] text-muted-foreground">
          {state.autoConnect ? "开仓自动连接已开启" : "开仓自动连接未开启"}
        </span>
        <HomeCardLink onClick={() => setActiveView("bluetooth")}>蓝牙页</HomeCardLink>
      </div>
    </div>
  );
}

function BatteryCell({
  label,
  value,
  charging,
}: {
  label: string;
  value: number | null;
  charging?: boolean | null;
}) {
  return (
    <div className="rounded-md bg-muted/50 px-2 py-1.5 text-center">
      <p className="text-[0.6rem] text-muted-foreground">{label}</p>
      <p className="text-xs font-semibold tabular-nums">
        {value === null ? "—" : `${value}%`}
        {charging ? <span className="ml-0.5 text-[0.6rem] text-brand">⚡</span> : null}
      </p>
    </div>
  );
}
