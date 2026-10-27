import { BatteryCharging, BluetoothOff, Headphones } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { useEarbuds } from "@/hooks/use-earbuds";
import { displayDeviceName, oneBudUnavailable } from "@/lib/ble-format";
import type { NoiseMode } from "@/lib/ipc";

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

/**
 * 组合窗口左侧「耳机」小视图：只读摘要（名称 / 阶段 / 电量 / 降噪标签）。
 *
 * 与首页的 `BluetoothCard` 同源数据，但**不渲染任何跳转入口**——首页那张卡底部的
 * 「蓝牙页」靠 `useAppStore.setActiveView`，而组合窗口是独立 WebView，改的是本窗口
 * 那份没人看的视图状态，点下去毫无反馈。完整操作请回主窗口的「蓝牙」页。
 */
export function HubEarbudsWidget() {
  const { state, readBattery } = useEarbuds();

  if (!state) {
    return <p className="text-xs text-muted-foreground">正在读取耳机状态…</p>;
  }

  const ready = state.phase === "ready";
  const phaseLabel = PHASE_LABEL[state.phase] ?? state.phase;
  const name =
    state.name || state.address ? displayDeviceName(state.name, state.address ?? "") : "";
  const battery = state.battery;

  const tags: string[] = [];
  if (state.noiseMode) {
    tags.push(NOISE_LABEL[state.noiseMode]);
    if (state.adaptiveAnc) tags.push("自适应");
  }
  if (state.gameMode) tags.push("游戏模式");
  if (state.dualDevice) tags.push("双设备");

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-start justify-between gap-2">
        <div className="flex min-w-0 items-center gap-1.5">
          {ready ? (
            <Headphones className="size-3.5 shrink-0 text-brand" aria-hidden />
          ) : (
            <BluetoothOff className="size-3.5 shrink-0 text-muted-foreground" aria-hidden />
          )}
          <span className="truncate text-xs font-medium">{name || "未连接耳机"}</span>
        </div>
        <Badge variant={ready ? "secondary" : "outline"} className="shrink-0">
          {phaseLabel}
        </Badge>
      </div>

      {ready && battery ? (
        <div className="grid grid-cols-3 gap-1.5">
          <BatteryCell label="左耳" value={battery.left} charging={battery.leftCharging} />
          <BatteryCell label="右耳" value={battery.right} charging={battery.rightCharging} />
          <BatteryCell label="仓" value={battery.case} />
        </div>
      ) : null}

      {ready && !battery ? (
        <Button variant="outline" size="xs" className="self-start" onClick={() => void readBattery()}>
          <BatteryCharging data-icon="inline-start" />
          读取电量
        </Button>
      ) : null}

      {ready && tags.length > 0 ? (
        <div className="flex flex-wrap gap-1">
          {tags.map((tag) => (
            <Badge key={tag} variant="secondary">
              {tag}
            </Badge>
          ))}
        </div>
      ) : null}

      {ready && oneBudUnavailable(battery) ? (
        <p className="text-[0.65rem] text-muted-foreground">
          有一只耳机没有上报电量；只有一只在线时，降噪梯度可能切换不动。
        </p>
      ) : null}

      {state.error ? (
        <p className="text-[0.65rem] leading-relaxed break-words text-destructive">{state.error}</p>
      ) : null}

      {!ready && state.activeAddress ? (
        <p className="text-[0.65rem] leading-relaxed text-muted-foreground">
          自动连接目标：{displayDeviceName(state.name, state.activeAddress)}
          {state.autoConnect ? "，取出耳机后会自动连接。" : "（自动连接未开启）。"}
        </p>
      ) : null}
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