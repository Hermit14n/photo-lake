import { useQuery } from "@tanstack/react-query";
import { Table2 } from "lucide-react";
import { useState, type ReactNode } from "react";
import type { EChartsOption } from "echarts";
import { axis, base, Chart, useTheme, type Theme } from "../components/Chart";
import { FilterBar } from "../components/FilterBar";
import { Card, Empty, Spinner } from "../components/ui";
import { api, toQuery, type Filters } from "../lib/api";
import { navigate, setParams, type Route } from "../lib/route";

type Dash = {
  summary: Record<string, number | string | null>;
  per_month: { month: string; photos: number; birds: number }[];
  per_day: { day: string; photos: number; birds: number }[];
  by_hour: { hour: number; photos: number; birds: number }[];
  by_weekday: { dow: number; photos: number }[];
  top_species: { code: string; name: string; sci: string; photos: number; confirmed: number; first_seen: string; last_seen: string }[];
  life_list: { day: string; new_species: number; total: number }[];
  species_by_month: { month: string; species: number }[];
  cameras: { name: string; photos: number }[];
  lenses: { name: string; photos: number }[];
  focal: { bucket: number; photos: number; birds: number }[];
  iso: { bucket: number; photos: number }[];
  places: { name: string; photos: number; birds: number; lat: number; lon: number }[];
};

const BAR = { barMaxWidth: 28 };
const TOP_ROUND = { borderRadius: [4, 4, 0, 0] as number[] };

export function Analytics({ route, filters }: { route: Route; filters: Filters }) {
  const t = useTheme();
  const q = useQuery({ queryKey: ["analytics", filters], queryFn: () => api.get<Dash>(`/analytics${toQuery(filters)}`), placeholderData: (p) => p });
  const d = q.data;
  const drill = (f: Partial<Filters>) => navigate("library", { ...filters, ...f } as Record<string, string>);

  return (
    <div className="flex h-full flex-col">
      <div className="border-b border-line px-4 py-3">
        <FilterBar filters={filters} onChange={(c) => setParams(route, c)} />
      </div>
      <div className="flex-1 overflow-auto p-4">
        {!d ? (
          <div className="flex justify-center py-24"><Spinner /></div>
        ) : Number(d.summary.photos) === 0 ? (
          <Empty title="Nothing to chart yet">Import some photos, or loosen the filters.</Empty>
        ) : (
          <div className="mx-auto max-w-[1500px] space-y-4">
            <Stats s={d.summary} onBirds={() => drill({ category: "bird" })} onUndated={() => drill({ undated: "true" })} onUnlocated={() => drill({ has_location: "false" })} />
            <div className="grid gap-4 xl:grid-cols-2">
              <ChartCard title="Photos per month" table={d.per_month.map((r) => [r.month, r.photos - r.birds, r.birds])} cols={["Month", "General", "Bird"]}>
                <Chart height={280} option={monthly(t, d)} onClick={(p) => { const [y, m] = String(p.name).split("-"); drill({ year: y, month: String(Number(m)) }); }} />
              </ChartCard>
              <ChartCard title="Life list" subtitle="Cumulative species photographed" table={d.life_list.map((r) => [r.day, r.new_species, r.total])} cols={["Day", "New", "Total"]}>
                <Chart height={280} option={lifeList(t, d)} onClick={(p) => drill({ from: String(p.name), to: String(p.name) })} />
              </ChartCard>
              <ChartCard title="Top species" subtitle="Photos per species (yours or model-identified)" table={d.top_species.map((r) => [r.name, r.photos, r.confirmed])} cols={["Species", "Photos", "Confirmed"]}>
                <Chart height={Math.max(220, Math.min(d.top_species.length, 15) * 26 + 30)} option={topSpecies(t, d)} onClick={(p) => drill({ species: d.top_species.find((s) => s.name === p.name)?.code, category: "bird" })} />
              </ChartCard>
              <ChartCard title="When you shoot" subtitle="Photos by hour of day" table={d.by_hour.map((r) => [`${r.hour}:00`, r.photos - r.birds, r.birds])} cols={["Hour", "General", "Bird"]}>
                <Chart height={280} option={hourly(t, d)} onClick={(p) => drill({ hour: String(parseInt(String(p.name))) })} />
              </ChartCard>
              <ChartCard title="Shooting days" subtitle="Photos per day" table={d.per_day.map((r) => [r.day, r.photos, r.birds])} cols={["Day", "Photos", "Bird"]}>
                <Chart height={220} option={calendar(t, d)} onClick={(p) => { const day = (p.data as [string, number])[0]; drill({ from: day, to: day }); }} />
              </ChartCard>
              <ChartCard title="Species per month" table={d.species_by_month.map((r) => [r.month, r.species])} cols={["Month", "Species"]}>
                <Chart height={220} option={single(t, d.species_by_month.map((r) => r.month), d.species_by_month.map((r) => r.species), "Species", t.bird)} onClick={(p) => { const [y, m] = String(p.name).split("-"); drill({ year: y, month: String(Number(m)), category: "bird" }); }} />
              </ChartCard>
              <ChartCard title="Focal length" subtitle="Photos per 25 mm" table={d.focal.map((r) => [`${r.bucket}–${r.bucket + 24} mm`, r.photos - r.birds, r.birds])} cols={["Focal", "General", "Bird"]}>
                <Chart height={220} option={focal(t, d)} />
              </ChartCard>
              <ChartCard title="Lenses" table={d.lenses.map((r) => [r.name, r.photos])} cols={["Lens", "Photos"]}>
                <Chart height={Math.max(160, d.lenses.length * 28 + 20)} option={hbar(t, d.lenses.map((r) => r.name), d.lenses.map((r) => r.photos), t.general)} onClick={(p) => p.name !== "Unknown" && drill({ lens: String(p.name) })} />
              </ChartCard>
            </div>
            {d.places.length > 0 && (
              <Card title="Places">
                <table className="w-full text-sm">
                  <thead><tr className="text-left text-xs text-muted"><th className="py-1.5 font-medium">Place</th><th className="font-medium text-right">Photos</th><th className="font-medium text-right">With birds</th></tr></thead>
                  <tbody>
                    {d.places.map((p) => (
                      <tr key={p.name} className="cursor-pointer border-t border-line hover:bg-s2" onClick={() => drill({ place: p.name })}>
                        <td className="py-1.5 text-ink">{p.name}</td><td className="text-right tabular-nums text-ink2">{p.photos}</td><td className="text-right tabular-nums text-ink2">{p.birds}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </Card>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

function Stats({ s, onBirds, onUndated, onUnlocated }: { s: Dash["summary"]; onBirds: () => void; onUndated: () => void; onUnlocated: () => void }) {
  const n = (v: unknown) => Number(v ?? 0).toLocaleString();
  const tile = (label: string, value: ReactNode, sub?: ReactNode, onClick?: () => void) => (
    <button disabled={!onClick} onClick={onClick} className="rounded-xl border border-line bg-s1 p-4 text-left enabled:hover:border-accent/60">
      <div className="text-xs text-ink2">{label}</div>
      <div className="mt-1 text-2xl font-semibold tabular-nums text-ink">{value}</div>
      {sub && <div className="mt-0.5 text-xs text-muted">{sub}</div>}
    </button>
  );
  const range = s.first && s.last ? `${String(s.first).slice(0, 10)} → ${String(s.last).slice(0, 10)}` : undefined;
  return (
    <div className="grid grid-cols-2 gap-3 md:grid-cols-3 xl:grid-cols-6">
      {tile("Photos", n(s.photos), `${(Number(s.bytes ?? 0) / 1e9).toFixed(1)} GB`)}
      {tile("Bird photos", n(s.bird_photos), `${Math.round((Number(s.bird_photos) / Math.max(1, Number(s.photos))) * 100)}% of photos`, onBirds)}
      {tile("Likely species", n(s.likely_species), `${n(s.confirmed_species)} confirmed · ${n(s.species)} incl. uncertain`)}
      {tile("Days shooting", n(s.days_shot), range)}
      {tile("Without location", n(Number(s.photos) - Number(s.located)), "click to fix", onUnlocated)}
      {tile("Without date", n(s.undated), Number(s.undated) ? "click to fix" : undefined, Number(s.undated) ? onUndated : undefined)}
    </div>
  );
}

/** Card with a chart/table toggle (every chart has a table view). */
function ChartCard({ title, subtitle, children, table, cols }: { title: string; subtitle?: string; children: ReactNode; table: (string | number)[][]; cols: string[] }) {
  const [asTable, setAsTable] = useState(false);
  return (
    <Card
      title={<div>{title}{subtitle && <div className="text-xs font-normal text-muted">{subtitle}</div>}</div>}
      action={<button onClick={() => setAsTable((v) => !v)} className="rounded-md p-1 text-muted hover:bg-s2 hover:text-ink" title={asTable ? "Show chart" : "Show table"} aria-pressed={asTable}><Table2 size={15} /></button>}
    >
      {asTable ? (
        <div className="max-h-72 overflow-auto">
          <table className="w-full text-sm">
            <thead className="sticky top-0 bg-s1"><tr>{cols.map((c, i) => <th key={c} className={`py-1 text-xs font-medium text-muted ${i ? "text-right" : "text-left"}`}>{c}</th>)}</tr></thead>
            <tbody>{table.map((r, i) => <tr key={i} className="border-t border-line">{r.map((v, j) => <td key={j} className={`py-1 ${j ? "text-right tabular-nums text-ink2" : "text-ink"}`}>{v}</td>)}</tr>)}</tbody>
          </table>
        </div>
      ) : children}
    </Card>
  );
}

function stacked(t: Theme, cats: string[], general: number[], birds: number[], rotate = 0): EChartsOption {
  const gap = { borderColor: t.surface, borderWidth: 1 };
  return {
    ...base(t),
    tooltip: { ...base(t).tooltip, trigger: "axis", axisPointer: { type: "shadow", shadowStyle: { color: "rgba(128,128,128,0.12)" } } },
    xAxis: { type: "category", data: cats, ...axis(t, { axisLabel: { color: t.muted, fontSize: 11, rotate } }) },
    yAxis: { type: "value", ...axis(t), minInterval: 1 },
    series: [
      { name: "General", type: "bar", stack: "a", data: general, itemStyle: { color: t.general, ...gap }, ...BAR },
      { name: "Bird", type: "bar", stack: "a", data: birds, itemStyle: { color: t.bird, ...gap, ...TOP_ROUND }, ...BAR },
    ],
  };
}

function monthly(t: Theme, d: Dash) {
  return stacked(t, d.per_month.map((r) => r.month), d.per_month.map((r) => r.photos - r.birds), d.per_month.map((r) => r.birds));
}

function hourly(t: Theme, d: Dash) {
  const hours = Array.from({ length: 24 }, (_, h) => d.by_hour.find((r) => r.hour === h) ?? { hour: h, photos: 0, birds: 0 });
  return stacked(t, hours.map((h) => `${h.hour}:00`), hours.map((h) => h.photos - h.birds), hours.map((h) => h.birds));
}

function focal(t: Theme, d: Dash) {
  return stacked(t, d.focal.map((r) => `${r.bucket}`), d.focal.map((r) => r.photos - r.birds), d.focal.map((r) => r.birds));
}

function single(t: Theme, cats: string[], vals: number[], name: string, color: string): EChartsOption {
  return {
    ...base(t),
    legend: { show: false },
    tooltip: { ...base(t).tooltip, trigger: "axis", axisPointer: { type: "shadow", shadowStyle: { color: "rgba(128,128,128,0.12)" } } },
    xAxis: { type: "category", data: cats, ...axis(t) },
    yAxis: { type: "value", ...axis(t), minInterval: 1 },
    series: [{ name, type: "bar", data: vals, itemStyle: { color, ...TOP_ROUND }, ...BAR }],
  };
}

function hbar(t: Theme, cats: string[], vals: number[], color: string): EChartsOption {
  return {
    ...base(t),
    legend: { show: false },
    grid: { left: 8, right: 40, top: 4, bottom: 4, containLabel: true },
    tooltip: { ...base(t).tooltip, trigger: "item" },
    xAxis: { type: "value", ...axis(t), splitLine: { show: false }, axisLabel: { show: false } },
    yAxis: { type: "category", inverse: true, data: cats, ...axis(t, { axisLine: { show: false }, axisLabel: { color: t.text2, fontSize: 12, width: 220, overflow: "truncate" } }) },
    series: [{ type: "bar", data: vals, barMaxWidth: 16, itemStyle: { color, borderRadius: [0, 4, 4, 0] }, label: { show: true, position: "right", color: t.text2, fontSize: 11 } }],
  };
}

function topSpecies(t: Theme, d: Dash) {
  const top = d.top_species.slice(0, 15);
  return hbar(t, top.map((s) => s.name), top.map((s) => s.photos), t.bird);
}

function lifeList(t: Theme, d: Dash): EChartsOption {
  return {
    ...base(t),
    legend: { show: false },
    tooltip: {
      ...base(t).tooltip,
      trigger: "axis",
      axisPointer: { type: "line", lineStyle: { color: t.muted } },
      formatter: (ps: any) => {
        const p = Array.isArray(ps) ? ps[0] : ps;
        const r = d.life_list[p.dataIndex];
        return `${r.day}<br/><b>${r.total}</b> species${r.new_species ? ` (+${r.new_species} new)` : ""}`;
      },
    },
    xAxis: { type: "category", data: d.life_list.map((r) => r.day), ...axis(t), boundaryGap: false },
    yAxis: { type: "value", ...axis(t), minInterval: 1 },
    series: [{ type: "line", step: "end", data: d.life_list.map((r) => r.total), showSymbol: d.life_list.length < 40, symbolSize: 8, lineStyle: { width: 2, color: t.bird }, itemStyle: { color: t.bird, borderColor: t.surface, borderWidth: 2 }, areaStyle: { color: t.bird, opacity: 0.08 } }],
  };
}

function calendar(t: Theme, d: Dash): EChartsOption {
  const days = d.per_day;
  const last = days.length ? days[days.length - 1].day : new Date().toISOString().slice(0, 10);
  const end = new Date(last);
  const start = new Date(end);
  start.setFullYear(end.getFullYear() - 1);
  start.setDate(start.getDate() + 1);
  const max = Math.max(1, ...days.map((r) => r.photos));
  return {
    ...base(t),
    tooltip: { ...base(t).tooltip, formatter: (p: any) => `${p.data[0]}<br/><b>${p.data[1]}</b> photos` },
    visualMap: { show: false, min: 0, max, inRange: { color: ["#cde2fb", "#86b6ef", "#3987e5", "#1c5cab", "#0d366b"] } },
    calendar: {
      range: [start.toISOString().slice(0, 10), last],
      top: 24, left: 36, right: 8, cellSize: ["auto", 16],
      itemStyle: { color: t.surface, borderColor: t.surface, borderWidth: 2 },
      splitLine: { show: false },
      dayLabel: { color: t.muted, fontSize: 10, firstDay: 1, nameMap: ["S", "M", "T", "W", "T", "F", "S"] },
      monthLabel: { color: t.muted, fontSize: 10 },
      yearLabel: { show: false },
    },
    series: [{ type: "heatmap", coordinateSystem: "calendar", data: days.map((r) => [r.day, r.photos]), itemStyle: { borderRadius: 3 } }],
  };
}

