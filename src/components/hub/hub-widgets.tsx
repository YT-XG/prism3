import { useEarbuds } from "@/hooks/use-earbuds";
import { useMouseBattery } from "@/hooks/use-mouse-battery";
import { useWirelessBatteries } from "@/hooks/use-wireless-batteries";
import { connectedDevices } from "@/lib/devices";

import { DeviceSummaryList } from "@/components/devices/device-summary-list";

/**
 * 组合窗口左栏的「设备」小视图：已连接设备的只读摘要（耳机 + 2.4G 鼠标 + 多品牌外设）。
 *
 * 列表本体与桌面小组件共用 `DeviceSummaryList`（同一份电量条版式），这里只负责取数。
 * **不渲染任何跳转入口**：组合窗口是独立 WebView，
 * `useAppStore.setActiveView` 改的是本窗口那份没人看的视图状态，点下去毫无反馈。
 * 完整操作请回主窗口的「设备」页。
 */
export function HubDevicesWidget({ active }: { active: boolean }) {
  const earbuds = useEarbuds();
  // 组合窗口是隐藏而非卸载：失焦时停表，别让后台窗口还在按 30s / 60s 读 HID。
  const mouse = useMouseBattery(active);
  const wireless = useWirelessBatteries(active);
  const devices = connectedDevices(earbuds.state, mouse, wireless);

  return (
    <DeviceSummaryList
      devices={devices}
      earbudError={earbuds.state?.error ?? null}
      onReadEarbuds={() => void earbuds.readBattery()}
    />
  );
}
