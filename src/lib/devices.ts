import type { BatteryReading, BleState, MouseBattery, WirelessBattery } from "@/lib/ipc";
import { displayDeviceName } from "@/lib/ble-format";

/**
 * 「设备」页与组合窗口共用的一台**已连接**设备。
 *
 * 这是一个判别联合而不是一个大而全的结构：耳机报左右耳 + 充电仓三档电量，鼠标只有
 * 一个百分比，硬塞进同一个电池模型反而会让两边都要写「取不到就 –」的兜底。
 */
export type ConnectedDevice =
  | {
      kind: "earbuds";
      /** React key / 列表去重用；同类设备只有一台。 */
      id: "earbuds";
      name: string;
      address: string;
      battery: BatteryReading | null;
      /** 降噪 / 自适应 / 游戏模式 / 双设备等状态标签。 */
      tags: string[];
    }
  | {
      kind: "mouse";
      id: "mouse";
      name: string;
      /** 「2.4G 接收器」/「有线」——鼠标的传输方式。 */
      subtitle: string;
      /** `null` = 未上报（有线 / 休眠 / 确实没有信息）。 */
      percent: number | null;
      charging: boolean;
      wireless: boolean;
    }
  | {
      /** 多品牌无线外设（Logitech / CompX / 标准 HID），只读电量。 */
      kind: "wireless";
      id: string;
      name: string;
      /** 设备类型：决定图标与排序。 */
      category: WirelessBattery["kind"];
      /** 「2.4G 接收器」/「USB 有线」/「USB HID」。 */
      subtitle: string;
      source: string;
      /** `null` = 这次没读到（休眠 / 有线 / 不支持），不是 0%。 */
      percent: number | null;
      charging: boolean | null;
      /** 百分比是否由电压估算（界面显示「约」）。 */
      approx: boolean;
    };

/**
 * 把数据源投影成「已连接的设备」列表。
 *
 * 纯函数：不做订阅、不发请求，页面与组合窗口各自持有 `useEarbuds()` / `useMouseBattery()` /
 * `useWirelessBatteries()` 后调用它——**不要**把这些 hook 塞进这里，否则两个窗口会各自
 * 多一份 BLE 事件订阅与 HID 轮询。
 *
 * 只收「此刻真的连着」的设备：耳机看 `phase === "ready"`，鼠标看 `connected`，
 * 多品牌外设看枚举结果（枚举只返回在场接口，休眠设备可能直接不出现）。
 * 未连接但已记住的耳机在「已记住的耳机」卡片里，不混进这张列表。
 */
export function connectedDevices(
  earbuds: BleState | null,
  mouse: MouseBattery,
  wireless: WirelessBattery[] = [],
): ConnectedDevice[] {
  const devices: ConnectedDevice[] = [];

  if (earbuds?.phase === "ready") {
    devices.push({
      kind: "earbuds",
      id: "earbuds",
      name: earbuds.name
        ? displayDeviceName(earbuds.name, earbuds.address ?? earbuds.name)
        : "已连接耳机",
      address: earbuds.address ?? "地址未知",
      battery: earbuds.battery,
      tags: earbudsTags(earbuds),
    });
  }

  if (mouse.connected) {
    devices.push({
      kind: "mouse",
      id: "mouse",
      name: "ML903",
      subtitle: mouse.wireless ? "2.4G 接收器" : "有线",
      percent: mouse.percent,
      charging: mouse.charging,
      wireless: mouse.wireless,
    });
  }

  for (const battery of wireless) {
    devices.push({
      kind: "wireless",
      id: battery.id,
      name: battery.name,
      category: battery.kind,
      subtitle: battery.transport,
      source: battery.source,
      percent: battery.percent,
      charging: battery.charging,
      approx: battery.approx,
    });
  }

  return devices;
}

/** 耳机状态标签：只列**设备真的在报**的项，推导值（如本地记忆的自适应）跟随降噪一起出现。 */
function earbudsTags(state: BleState): string[] {
  const tags: string[] = [];
  if (state.noiseMode) {
    tags.push(
      { anc: "降噪", normal: "正常", wind: "抗风噪", transparency: "通透" }[state.noiseMode],
    );
    if (state.adaptiveAnc) tags.push("自适应");
  }
  if (state.gameMode) tags.push("游戏模式");
  if (state.dualDevice) tags.push("双设备");
  return tags;
}

/** 电量一句话摘要（列表、组合窗口共用）：未报告的项显示「–」，绝不落成 0%。 */
export function earbudsBatterySummary(battery: BatteryReading | null): string {
  if (!battery) return "电量未知";
  const percent = (value: number | null) => (value === null ? "–" : `${value}%`);
  return `左 ${percent(battery.left)} · 右 ${percent(battery.right)} · 仓 ${percent(battery.case)}`;
}

/** 鼠标电量一句话摘要；`null` 时说明「未上报」而不是 0%。 */
export function mouseBatterySummary(device: Extract<ConnectedDevice, { kind: "mouse" }>): string {
  if (device.percent === null) {
    return device.wireless ? "未上报电量（可能已休眠）" : "有线模式不读电量";
  }
  return device.charging ? `${device.percent}% · 充电中` : `${device.percent}%`;
}

/** 多品牌无线外设电量一句话摘要；充电状态未知时只显示百分比。 */
export function wirelessBatterySummary(
  device: Extract<ConnectedDevice, { kind: "wireless" }>,
): string {
  if (device.percent === null) {
    return "未上报电量（可能已休眠）";
  }
  const value = `${device.approx ? "约 " : ""}${device.percent}%`;
  return device.charging === true ? `${value} · 充电中` : value;
}
