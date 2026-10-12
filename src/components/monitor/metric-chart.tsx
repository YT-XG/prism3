import { useEffect, useRef } from "react";

import uPlot from "uplot";

import "uplot/dist/uPlot.min.css";

export interface ChartSeries {
  label: string;
  /** 折线颜色，必须是具体色值（canvas 无法解析 CSS 变量）。 */
  color: string;
  /** 面积填充色（含透明度），可选。 */
  fill?: string;
}

interface MetricChartProps {
  xs: number[];
  ys: (number | null)[][];
  series: ChartSeries[];
  height?: number;
  /** 固定 y 轴上界（如 CPU/内存固定 0-100）；不传则自适应。 */
  max?: number;
}

/** uPlot 的 React 封装：迷你折线，隐藏坐标轴、图例与交互。 */
export function MetricChart({ xs, ys, series, height = 88, max }: MetricChartProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const plotRef = useRef<uPlot | null>(null);
  // 颜色 / 系列变化时需重建实例（canvas 取不到 CSS 变量）。
  const seriesKey = series
    .map((item) => `${item.label}:${item.color}:${item.fill ?? ""}`)
    .join("|");

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const plotSeries: uPlot.Series[] = [
      {},
      ...series.map((item) => ({
        label: item.label,
        stroke: item.color,
        fill: item.fill,
        width: 2,
        points: { show: false },
      })),
    ];

    const plot = new uPlot(
      {
        width: container.clientWidth || 320,
        height,
        legend: { show: false },
        cursor: { show: false },
        padding: [6, 0, 4, 0],
        scales: {
          x: { time: false },
          ...(max !== undefined ? { y: { range: [0, max] as [number, number] } } : {}),
        },
        axes: [{ show: false }, { show: false }],
        series: plotSeries,
      },
      [xs, ...ys] as uPlot.AlignedData,
      container,
    );
    plotRef.current = plot;

    // uPlot 的 CSS 含 `width: min-content`，必须按容器实测宽度调 setSize。
    const observer = new ResizeObserver((entries) => {
      const width = Math.floor(entries[0]?.contentRect.width ?? container.clientWidth);
      if (width > 0 && width !== plot.width) {
        plot.setSize({ width, height });
      }
    });
    observer.observe(container);

    return () => {
      observer.disconnect();
      plot.destroy();
      plotRef.current = null;
    };
  }, [seriesKey, height, max]);

  useEffect(() => {
    plotRef.current?.setData([xs, ...ys] as uPlot.AlignedData);
  }, [xs, ys]);

  return <div ref={containerRef} className="w-full" style={{ height }} />;
}