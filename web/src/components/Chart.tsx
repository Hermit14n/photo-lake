import * as echarts from "echarts";
import { useEffect, useRef, useState } from "react";

/** Read the current theme tokens (so charts follow light/dark). */
export function useTheme() {
  const read = () => {
    const s = getComputedStyle(document.documentElement);
    const v = (n: string) => s.getPropertyValue(n).trim();
    return {
      surface: v("--surface-1"),
      text: v("--text-primary"),
      text2: v("--text-secondary"),
      muted: v("--text-muted"),
      grid: v("--grid"),
      general: v("--series-general"),
      bird: v("--series-bird"),
      border: v("--border"),
    };
  };
  const [t, setT] = useState(read);
  useEffect(() => {
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const on = () => setT(read());
    mq.addEventListener("change", on);
    const mo = new MutationObserver(on);
    mo.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
    return () => {
      mq.removeEventListener("change", on);
      mo.disconnect();
    };
  }, []);
  return t;
}
export type Theme = ReturnType<typeof useTheme>;

/** Shared axis/tooltip styling: recessive grid, text in text tokens. */
export function base(t: Theme): echarts.EChartsOption {
  return {
    animationDuration: 300,
    textStyle: { fontFamily: "Inter, system-ui, sans-serif", color: t.text2 },
    grid: { left: 8, right: 16, top: 28, bottom: 8, containLabel: true },
    tooltip: {
      backgroundColor: t.surface,
      borderColor: t.border,
      textStyle: { color: t.text, fontSize: 12 },
      extraCssText: "border-radius:8px;box-shadow:0 8px 24px rgba(0,0,0,.25)",
    },
    legend: { top: 0, right: 0, icon: "roundRect", itemWidth: 10, itemHeight: 10, textStyle: { color: t.text2 } },
  };
}

export function axis(t: Theme, extra: Record<string, unknown> = {}) {
  return {
    axisLine: { lineStyle: { color: t.border } },
    axisTick: { show: false },
    axisLabel: { color: t.muted, fontSize: 11 },
    splitLine: { lineStyle: { color: t.grid } },
    ...extra,
  };
}

export function Chart({ option, height = 260, onClick }: { option: echarts.EChartsOption; height?: number; onClick?: (p: echarts.ECElementEvent) => void }) {
  const el = useRef<HTMLDivElement>(null);
  const chart = useRef<echarts.ECharts | null>(null);
  const click = useRef(onClick);
  click.current = onClick;

  useEffect(() => {
    if (!el.current) return;
    const c = echarts.init(el.current, undefined, { renderer: "svg" });
    chart.current = c;
    c.on("click", (p) => click.current?.(p));
    const ro = new ResizeObserver(() => c.resize());
    ro.observe(el.current);
    return () => {
      ro.disconnect();
      c.dispose();
    };
  }, []);
  useEffect(() => {
    chart.current?.setOption(option, true);
  }, [option]);
  return <div ref={el} style={{ height, cursor: onClick ? "pointer" : undefined }} />;
}
