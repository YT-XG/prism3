import { useCallback, useMemo, useState, type CSSProperties } from "react";

import type { PanelPage } from "@/components/quick-panel/panel-pager";
import { PanelPager } from "@/components/quick-panel/panel-pager";
import { QuickPanelBluetooth } from "@/components/quick-panel/quick-panel-bluetooth";
import { QuickPanelClipboard } from "@/components/quick-panel/quick-panel-clipboard";
import { QuickPanelSearch } from "@/components/quick-panel/quick-panel-search";
import { useQuickPanelBody, CLIP_BY_EDGE } from "@/hooks/use-quick-panel-body";
import {
  PANEL_WIDTH,
  PANEL_WIDTH_TRANSITION,
  PANEL_WIDTH_WIDE,
} from "@/hooks/use-quick-panel-window";
import { useSystemMonitor } from "@/hooks/use-system-monitor";
import { formatBytes, formatPercent, formatRate, formatUptime } from "@/lib/format";
import type { SystemSnapshot } from "@/lib/ipc";
import { cn } from "@/lib/utils";

import { Bluetooth, ClipboardList, Gauge, Info, MousePointerClick, Search } from "lucide-react";

const APP_VERSION = "0.1.0";
/** 搜索页的页面 id（分页器按 id 记忆停留页，改名会丢记忆）。 */
const SEARCH_PAGE_ID = "search";
/** 蓝牙页的页面 id（同上：改名会丢停留页记忆）。 */
const BLUETOOTH_PAGE_ID = "bluetooth";
/** 剪贴板页的页面 id（同上）。 */
const CLIPBOARD_PAGE_ID = "clipboard";

/**
 * 快捷小窗面板本体：跑在独立窗口 `quick-panel-body` 中，高度恒为 `PANEL_HEIGHT`。
 *
 * **窗口尺寸恒为最大页宽（`PANEL_WINDOW_WIDTH × PANEL_HEIGHT`），可见期间从不
 * 缩放**：WebView 内容跟不上窗口缩放，缩放瞬间旧帧会贴新视口左上角被画出一两
 * 帧（面板整体平移 / 瘦成一条再弹回，见 docs/quick-panel-flash-postmortem.md）。
 * 换页调宽调窄只做面板盒子自身的宽度过渡（贴灰条一侧），窗口几何完全不动；
 * 各页内容按各自宽度定宽，过渡期间文字不重排。窄页时窗口比盒子宽出的部分是
 * 透明死区（贴远侧）：不参与悬停（悬停挂在盒子上），但面板展开期间会拦截
 * 该区域的点击。
 *
 * 展开/收起只做 CSS 动画（从灰条一侧擦拭展开 + 淡入，收起反向），同样不动窗口。
 * 内容在收起状态下保持「不可见」而非卸载，因此系统监控用 `phase` 显式停表，
 * 避免收起后继续轮询。
 */
export function QuickPanelBody() {
  const {
    phase,
    edge,
    contentWidth,
    focusMode,
    requestFocus,
    releaseFocus,
    dismiss,
    requestWidth,
    rootProps,
    bodyProps,
  } = useQuickPanelBody();
  // 面板只展示当前值、不画趋势图，故关闭历史缓冲，省去每秒的数组克隆。
  const { snapshot } = useSystemMonitor({ enabled: phase === "open", trackHistory: false });

  /** 当前停留的页面 id：搜索页只在被选中且面板展开时才发起查询。 */
  const [activePageId, setActivePageId] = useState("overview");

  const pages: PanelPage[] = useMemo(
    () => [
      {
        id: "overview",
        label: "概览",
        icon: Gauge,
        content: <OverviewPage snapshot={snapshot} />,
      },
      {
        id: BLUETOOTH_PAGE_ID,
        label: "蓝牙",
        icon: Bluetooth,
        // 用默认页宽：设备列表 + 设备设置两级都在一页内，窄页足够；面板窗口恒为
        // PANEL_WINDOW_WIDTH 且可见期间不缩放，页宽差异只是盒子自身的 CSS 宽度。
        content: (
          <QuickPanelBluetooth active={phase === "open" && activePageId === BLUETOOTH_PAGE_ID} />
        ),
      },
      {
        id: CLIPBOARD_PAGE_ID,
        label: "剪贴板",
        icon: ClipboardList,
        // 与搜索页同为最大页宽：行里要放预览 + 时间 + 行内按钮，窄页放不下。
        panelWidth: PANEL_WIDTH_WIDE,
        content: (
          <QuickPanelClipboard
            active={phase === "open" && activePageId === CLIPBOARD_PAGE_ID}
            focused={focusMode}
            onRequestFocus={requestFocus}
            onReleaseFocus={releaseFocus}
            onDismiss={dismiss}
          />
        ),
      },
      {
        id: SEARCH_PAGE_ID,
        label: "搜索",
        icon: Search,
        // 与帮助页同为最大页宽：面板窗口恒为 PANEL_WINDOW_WIDTH，不越界也不会被裁。
        panelWidth: PANEL_WIDTH_WIDE,
        content: (
          <QuickPanelSearch
            active={phase === "open" && activePageId === SEARCH_PAGE_ID}
            focused={focusMode}
            onRequestFocus={requestFocus}
            onDismiss={dismiss}
          />
        ),
      },
      {
        id: "help",
        label: "帮助",
        icon: MousePointerClick,
        panelWidth: PANEL_WIDTH_WIDE,
        content: <HelpPage />,
      },
      { id: "about", label: "关于", icon: Info, content: <AboutPage /> },
    ],
    [activePageId, dismiss, focusMode, phase, releaseFocus, requestFocus, snapshot],
  );

  const handleActiveChange = useCallback(
    (page: PanelPage) => {
      setActivePageId(page.id);
      // 离开搜索页就退出输入聚焦模式：否则键盘焦点会留在看不见的输入框上。
      if (page.id !== SEARCH_PAGE_ID) void releaseFocus();
      requestWidth(page.panelWidth ?? PANEL_WIDTH);
    },
    [releaseFocus, requestWidth],
  );

  // 面板盒子自身以宽度过渡加宽 / 收窄，始终贴住灰条一侧：远侧边缘移出或收回，
  // 内容按各页自己的宽度定宽（见 `PanelPager`），因此过渡期间没有任何文字重排。
  // 窗口尺寸恒定不动（见组件注释），宽度变化纯属盒子的 CSS 过渡。
  const isLeft = edge === "left";

  return (
    <div className="relative h-full w-full">
      <div
        {...rootProps}
        {...bodyProps}
        style={
          {
            "--qp-clip": CLIP_BY_EDGE[edge],
            width: contentWidth,
            transitionDuration: `${PANEL_WIDTH_TRANSITION}ms`,
          } as CSSProperties
        }
        className={cn(
          "quick-panel-body absolute inset-y-0 flex flex-col overflow-hidden rounded-xl border border-border bg-background text-foreground transition-[width] ease-out",
          isLeft ? "left-0" : "right-0",
        )}
      >
        <div className="flex h-9 shrink-0 items-center border-b border-border/60 px-3">
          <span className="text-xs font-medium text-muted-foreground">快捷小窗</span>
        </div>

        <PanelPager
          pages={pages}
          defaultPanelWidth={PANEL_WIDTH}
          anchor={isLeft ? "start" : "end"}
          onActiveChange={handleActiveChange}
        />
      </div>
    </div>
  );
}

function PageHeading({ title, hint }: { title: string; hint: string }) {
  return (
    <div className="flex flex-col gap-0.5">
      <h2 className="font-heading text-sm font-medium">{title}</h2>
      <p className="text-xs text-muted-foreground">{hint}</p>
    </div>
  );
}

function Meter({
  label,
  value,
  percent,
  tone,
}: {
  label: string;
  value: string;
  percent: number;
  tone: "cpu" | "mem";
}) {
  const clamped = Math.min(100, Math.max(0, percent));
  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex items-baseline justify-between gap-3 text-xs">
        <span className="text-muted-foreground">{label}</span>
        <span className="font-heading text-sm font-medium tabular-nums">{value}</span>
      </div>
      <div className="h-1.5 w-full overflow-hidden rounded-full bg-muted">
        <div
          className={cn(
            "h-full rounded-full transition-[width] duration-500 ease-out",
            tone === "cpu" ? "bg-blue-500" : "bg-violet-500",
          )}
          style={{ width: `${clamped}%` }}
        />
      </div>
    </div>
  );
}

function StatRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-baseline justify-between gap-3 text-xs">
      <span className="text-muted-foreground">{label}</span>
      <span className="truncate font-medium tabular-nums">{value}</span>
    </div>
  );
}

/** 硬件型号：窄面板下型号通常较长，标签在上、型号在下并允许换行。 */
function HardwareRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex flex-col gap-0.5 text-xs">
      <span className="text-muted-foreground">{label}</span>
      <span className="break-words font-medium">{value}</span>
    </div>
  );
}

function OverviewPage({ snapshot }: { snapshot: SystemSnapshot | null }) {
  const cpu = snapshot?.cpuUsage ?? null;
  const memPercent =
    snapshot && snapshot.memTotal > 0 ? (snapshot.memUsed / snapshot.memTotal) * 100 : 0;

  return (
    <div className="flex flex-col gap-3.5">
      <PageHeading title="系统概览" hint="展开时每 1 秒刷新" />

      <Meter
        label="CPU"
        value={cpu === null ? "—" : formatPercent(cpu)}
        percent={cpu ?? 0}
        tone="cpu"
      />
      <Meter
        label={snapshot ? `内存 ${formatBytes(snapshot.memUsed)} / ${formatBytes(snapshot.memTotal)}` : "内存"}
        value={snapshot ? formatPercent(memPercent) : "—"}
        percent={memPercent}
        tone="mem"
      />

      <div className="grid grid-cols-2 gap-x-4 gap-y-1.5 rounded-lg bg-muted/50 p-2.5">
        <StatRow label="↓ 下载" value={formatRate(snapshot?.netRxPerSec ?? 0)} />
        <StatRow label="↑ 上传" value={formatRate(snapshot?.netTxPerSec ?? 0)} />
        <StatRow label="主机" value={snapshot?.hostname ?? "—"} />
        <StatRow label="开机" value={snapshot ? formatUptime(snapshot.uptimeSecs) : "—"} />
      </div>

      <div className="flex flex-col gap-2.5">
        <span className="text-xs font-medium text-muted-foreground">硬件</span>
        {snapshot ? (
          <div className="flex flex-col gap-2">
            {snapshot.cpuBrand && <HardwareRow label="处理器" value={snapshot.cpuBrand} />}
            {snapshot.motherboard && <HardwareRow label="主板" value={snapshot.motherboard} />}
            {snapshot.systemModel && <HardwareRow label="整机" value={snapshot.systemModel} />}
            {snapshot.gpus.map((gpu) => (
              <HardwareRow
                key={`${gpu.name}-${gpu.memory}`}
                label="显卡"
                value={gpu.memory > 0 ? `${gpu.name} · ${formatBytes(gpu.memory)}` : gpu.name}
              />
            ))}
            {snapshot.memoryModules.map((memory) => (
              <HardwareRow
                key={`${memory.partNumber}-${memory.capacity}`}
                label="内存"
                value={[
                  memory.memoryType,
                  memory.speed > 0 ? `${memory.speed}MHz` : "",
                  formatBytes(memory.capacity),
                ]
                  .filter(Boolean)
                  .join(" · ")}
              />
            ))}
          </div>
        ) : (
          <p className="text-xs text-muted-foreground">正在读取硬件信息…</p>
        )}
      </div>

      {snapshot && snapshot.disks.length > 0 && (
        <div className="flex flex-col gap-2.5">
          <span className="text-xs font-medium text-muted-foreground">磁盘</span>
          {snapshot.disks.map((disk) => {
            const used = Math.max(0, disk.total - disk.available);
            const percent = disk.total > 0 ? (used / disk.total) * 100 : 0;
            const label = disk.mountPoint || disk.name || "磁盘";
            return (
              <div key={disk.mountPoint} className="flex flex-col gap-1.5">
                <div className="flex items-baseline justify-between gap-3 text-xs">
                  <span className="font-medium">{label}</span>
                  <span className="text-muted-foreground tabular-nums">
                    {formatPercent(percent)} · 剩余 {formatBytes(disk.available)}
                  </span>
                </div>
                <div className="h-1.5 w-full overflow-hidden rounded-full bg-muted">
                  <div
                    className="h-full rounded-full bg-emerald-500 transition-[width] duration-500 ease-out"
                    style={{ width: `${Math.min(100, percent)}%` }}
                  />
                </div>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}

const HELP_ITEMS: { keys: string; desc: string }[] = [
  { keys: "拖动小灰条", desc: "沿屏幕左 / 右 / 上边缘移动，靠近边缘自动吸附（不含底部）" },
  { keys: "点击左侧图标", desc: "直达页面，选中高亮胶囊平滑跟随" },
  { keys: "底部宫格按钮", desc: "展开全部页面，一键跳转" },
  { keys: "触控板左右滑 / 滚轮", desc: "逐页切换；内容滚到边缘后继续滚也能翻页" },
  { keys: "移开鼠标 1 秒", desc: "展开面板自动收起，只剩小灰条" },
  {
    keys: "搜索页点击输入框",
    desc: "临时接管键盘焦点（面板平时不抢焦点）；打字期间不会自动收起，Esc 退出并把焦点还给原应用",
  },
];

function HelpPage() {
  return (
    <div className="flex flex-col gap-3.5">
      <PageHeading title="使用提示" hint="当前页会被记住，下次展开时回到这里" />
      <ul className="flex flex-col gap-2.5">
        {HELP_ITEMS.map((item) => (
          <li key={item.keys} className="flex flex-col gap-0.5">
            <span className="text-xs font-medium">{item.keys}</span>
            <span className="text-xs text-muted-foreground">{item.desc}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}

function AboutPage() {
  return (
    <div className="flex flex-col gap-3.5">
      <PageHeading title="关于" hint={`Prism3 桌面脚手架 · v${APP_VERSION}`} />
      <p className="text-xs leading-relaxed text-muted-foreground">
        快捷小窗是屏幕边缘的常驻入口：灰条始终可见，悬停时面板从灰条旁展开、离开自动收起。
        拖动灰条即可换边；左侧图标栏直达各页，内容纵向逐页滑过，底部宫格可一键跳转并记忆停留页面。
      </p>
      <div className="flex flex-col gap-1.5 rounded-lg bg-muted/50 p-2.5">
        <StatRow label="窗口" value="quick-panel + quick-panel-body" />
        <StatRow label="面板尺寸" value={`${PANEL_WIDTH} × 520（帮助页加宽至 ${PANEL_WIDTH_WIDE}）`} />
        <StatRow label="当前页面" value="由本地记忆恢复" />
      </div>
    </div>
  );
}
