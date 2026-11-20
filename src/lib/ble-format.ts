import type { BatteryReading } from "@/lib/ipc";

/** 设备名末尾的 BLE 后缀：连接后的 GATT Device Name 形如 `<产品名>_BLE`。 */
const BLE_SUFFIX = /\s*_ble\s*$/i;

/**
 * 耳机的展示名：去掉名字末尾的 `_BLE` 后缀（大小写不敏感，后缀前后允许空白）。
 *
 * 耳机**广播**不带名字（`name` 为 null 是正常现象，不要当错误），因此名字缺失时
 * 退回设备地址；名字只剩后缀时同样退回地址，避免渲染出一个空标题。
 */
export function displayDeviceName(name: string | null, address: string): string {
  const trimmed = name?.trim() ?? "";
  if (!trimmed) return address;

  const stripped = trimmed.replace(BLE_SUFFIX, "").trim();
  return stripped || address;
}

/**
 * 广播里厂商公司标识（蓝牙 SIG 分配号）的展示文本，形如 `0x5452` / `0x0B0D / 0x5452`。
 *
 * 设备页把它放在地址后面：同品牌换型号未必还是同一个号，看一眼比猜型号可靠；
 * 空列表给 `无厂商数据`，而不是留一段空白让人以为是渲染丢了。
 */
export function companyIdsText(companyIds: number[]): string {
  if (companyIds.length === 0) return "无厂商数据";
  return companyIds
    .map((id) => `0x${id.toString(16).toUpperCase().padStart(4, "0")}`)
    .join(" / ");
}

/**
 * 是否有一只耳机不在线（电量读数里那只耳机是 `null`）。
 *
 * ① 实测（用户，见协议笔记 §14.3 第 4 条）：**只有一只耳机在线时，耳机不接受降噪梯度的切换**——
 * 它照样响一声反馈，但值不变，回读也仍是旧值（两只都取出时切换正常）。所以「点了有反馈、
 * 值却不动」先按这个前提排查，别急着怀疑读回或界面。
 * ② 电量字节 `0xFF`（后端已映射为 `null`）表示「这只耳机没有上报」，这一点**很可能但未证实**——
 * 正因如此，本判定只用来给一句提示，**绝不用来禁用任何控件**（断错会把功能锁死）。
 *
 * 只看左右耳：充电仓不是耳机，且它「不在位」是常态，不能拿它当在线依据。
 */
export function oneBudUnavailable(battery: BatteryReading | null): boolean {
  if (!battery) return false;
  return battery.left === null || battery.right === null;
}
