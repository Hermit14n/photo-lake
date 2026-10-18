import { useInfiniteQuery } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import clsx from "clsx";
import { Bird, Check, ImageOff, MapPin, PencilLine, Star } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { BulkBar } from "../components/BulkBar";
import { FilterBar } from "../components/FilterBar";
import { Empty, Segmented, Spinner } from "../components/ui";
import { api, fmtDate, img, localDayKey, toQuery, useReadOnly, type Filters, type PhotoSummary } from "../lib/api";
import { setParams, type Route } from "../lib/route";

const PAGE = 400;
const GAP = 6;
const SIZES = { s: 140, m: 200, l: 300 } as const;

type Row = { kind: "header"; day: string; count: number; birds: number; ids: string[] } | { kind: "tiles"; items: PhotoSummary[] };

function dayLabel(day: string) {
  if (day === "undated") return "No date";
  const [y, m, d] = day.split("-").map(Number);
  return new Date(y, m - 1, d).toLocaleDateString(undefined, { weekday: "long", year: "numeric", month: "long", day: "numeric" });
}

export function Library({ route, filters }: { route: Route; filters: Filters }) {
  const [size, setSize] = useState<keyof typeof SIZES>(() => (localStorageGet("tile") as keyof typeof SIZES) || "m");
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const readOnly = useReadOnly();
  const lastClicked = useRef<string | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(1200);

  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setWidth(el.clientWidth - 32));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  useEffect(() => setSelected(new Set()), [JSON.stringify(filters)]); // eslint-disable-line react-hooks/exhaustive-deps

  const q = useInfiniteQuery({
    queryKey: ["photos", filters],
    queryFn: ({ pageParam }) =>
      api.get<{ items: PhotoSummary[]; total: number }>(`/photos${toQuery(filters, { limit: PAGE, offset: pageParam })}`),
    initialPageParam: 0,
    getNextPageParam: (last, all) => {
      const loaded = all.reduce((n, p) => n + p.items.length, 0);
      return loaded < last.total ? loaded : undefined;
    },
  });
  const items = useMemo(() => q.data?.pages.flatMap((p) => p.items) ?? [], [q.data]);
  const total = q.data?.pages[0]?.total ?? 0;
  const tile = SIZES[size];
  const perRow = Math.max(1, Math.floor((width + GAP) / (tile + GAP)));
  const tileW = (width - GAP * (perRow - 1)) / perRow;

  const rows = useMemo(() => {
    const out: Row[] = [];
    let i = 0;
    while (i < items.length) {
      const day = localDayKey(items[i].captured_at);
      let j = i;
      while (j < items.length && localDayKey(items[j].captured_at) === day) j++;
      const group = items.slice(i, j);
      out.push({ kind: "header", day, count: group.length, birds: group.filter((g) => g.category === "bird").length, ids: group.map((g) => g.photo_id) });
      for (let k = 0; k < group.length; k += perRow) out.push({ kind: "tiles", items: group.slice(k, k + perRow) });
      i = j;
    }
    return out;
  }, [items, perRow]);

  const virt = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: (i) => (rows[i].kind === "header" ? 48 : tileW + GAP),
    overscan: 6,
  });
  useEffect(() => virt.measure(), [tileW]); // eslint-disable-line react-hooks/exhaustive-deps

  const vItems = virt.getVirtualItems();
  useEffect(() => {
    const last = vItems[vItems.length - 1];
    if (last && last.index >= rows.length - 4 && q.hasNextPage && !q.isFetchingNextPage) q.fetchNextPage();
  }, [vItems, rows.length, q]);

  const click = (p: PhotoSummary, e: React.MouseEvent) => {
    if (e.shiftKey && lastClicked.current) {
      const a = items.findIndex((x) => x.photo_id === lastClicked.current);
      const b = items.findIndex((x) => x.photo_id === p.photo_id);
      const [lo, hi] = a < b ? [a, b] : [b, a];
      setSelected((s) => new Set([...s, ...items.slice(lo, hi + 1).map((x) => x.photo_id)]));
    } else if (e.metaKey || e.ctrlKey || selected.size > 0) {
      toggle(p.photo_id);
    } else {
      setParams(route, { photo: p.photo_id });
    }
    lastClicked.current = p.photo_id;
  };
  const toggle = (id: string) =>
    setSelected((s) => {
      const n = new Set(s);
      n.has(id) ? n.delete(id) : n.add(id);
      return n;
    });

  return (
    <div className="flex h-full flex-col">
      <div className="border-b border-line bg-bg px-4 py-3">
        <FilterBar
          filters={filters}
          onChange={(c) => setParams(route, c)}
          showSort
          extra={
            <div className="ml-auto flex items-center gap-3">
              <span className="text-sm tabular-nums text-ink2">{total.toLocaleString()} photos</span>
              <Segmented
                value={size}
                onChange={(v) => {
                  setSize(v);
                  localStorageSet("tile", v);
                }}
                options={[
                  { value: "s", label: "S" },
                  { value: "m", label: "M" },
                  { value: "l", label: "L" },
                ]}
              />
            </div>
          }
        />
      </div>
      <div ref={scrollRef} className="relative flex-1 overflow-auto px-4">
        {q.isLoading && (
          <div className="flex justify-center py-24">
            <Spinner />
          </div>
        )}
        {q.isError && <Empty title="Couldn't load photos">{String(q.error)}</Empty>}
        {q.isSuccess && items.length === 0 && (
          <Empty icon={<ImageOff size={36} />} title="No photos match">
            Try clearing some filters, or import a card from the Imports page.
          </Empty>
        )}
        <div style={{ height: virt.getTotalSize(), position: "relative" }}>
          {vItems.map((v) => {
            const row = rows[v.index];
            return (
              <div key={v.key} style={{ position: "absolute", top: 0, left: 0, right: 0, transform: `translateY(${v.start}px)` }}>
                {row.kind === "header" ? (
                  <div className="flex h-12 items-end gap-3 pb-2">
                    <button
                      className="text-base font-semibold text-ink hover:text-accent"
                      title="Show only this day"
                      onClick={() => row.day !== "undated" && setParams(route, { from: row.day, to: row.day, year: undefined, month: undefined })}
                    >
                      {dayLabel(row.day)}
                    </button>
                    <span className="text-xs text-muted">
                      {row.count} photo{row.count === 1 ? "" : "s"}
                      {row.birds > 0 && ` · ${row.birds} with birds`}
                    </span>
                    {selected.size > 0 && (
                      <button className="text-xs text-focus hover:underline" onClick={() => setSelected((s) => new Set([...s, ...row.ids]))}>
                        Select day
                      </button>
                    )}
                  </div>
                ) : (
                  <div className="flex" style={{ gap: GAP, paddingBottom: GAP }}>
                    {row.items.map((p) => (
                      <Tile key={p.photo_id} p={p} size={tileW} selected={selected.has(p.photo_id)} selecting={selected.size > 0} onClick={click} onToggle={toggle} />
                    ))}
                  </div>
                )}
              </div>
            );
          })}
        </div>
        {q.isFetchingNextPage && (
          <div className="flex justify-center py-6">
            <Spinner />
          </div>
        )}
      </div>
      {selected.size > 0 && !readOnly && <BulkBar ids={[...selected]} onDone={() => setSelected(new Set())} onSelectAll={() => setSelected(new Set(items.map((i) => i.photo_id)))} />}
    </div>
  );
}

function Tile({
  p,
  size,
  selected,
  selecting,
  onClick,
  onToggle,
}: {
  p: PhotoSummary;
  size: number;
  selected: boolean;
  selecting: boolean;
  onClick: (p: PhotoSummary, e: React.MouseEvent) => void;
  onToggle: (id: string) => void;
}) {
  return (
    <div
      role="button"
      tabIndex={0}
      onClick={(e) => onClick(p, e)}
      onKeyDown={(e) => e.key === "Enter" && onClick(p, e as unknown as React.MouseEvent)}
      title={`${p.original_filename} · ${fmtDate(p.captured_at)}`}
      className={clsx(
        "group relative cursor-pointer overflow-hidden rounded-lg bg-s2",
        selected && "ring-2 ring-accent ring-offset-2 ring-offset-bg",
      )}
      style={{ width: size, height: size }}
    >
      <img src={img(p.photo_id, "thumb")} loading="lazy" decoding="async" alt={p.species ?? p.original_filename} className="h-full w-full object-cover transition-transform duration-300 group-hover:scale-[1.03]" />
      <button
        onClick={(e) => {
          e.stopPropagation();
          onToggle(p.photo_id);
        }}
        aria-label={selected ? "Deselect" : "Select"}
        className={clsx(
          "absolute left-2 top-2 flex h-6 w-6 items-center justify-center rounded-full border-2 transition-opacity",
          selected ? "border-accent bg-accent text-white opacity-100" : "border-white/80 bg-black/30 text-transparent",
          selecting || selected ? "opacity-100" : "opacity-0 group-hover:opacity-100",
        )}
      >
        <Check size={14} strokeWidth={3} />
      </button>
      <div className="pointer-events-none absolute inset-x-0 bottom-0 flex items-end justify-between gap-1 bg-gradient-to-t from-black/75 via-black/30 to-transparent p-2 pt-8 text-white">
        <div className="min-w-0">
          {p.category === "bird" && (
            <div className="flex items-center gap-1 text-xs font-medium">
              <Bird size={13} className="shrink-0 text-[#ffb08a]" />
              <span className="truncate">{p.species ?? "Bird"}</span>
            </div>
          )}
        </div>
        <div className="flex shrink-0 items-center gap-1 text-white/80">
          {p.rating ? (
            <span className="flex items-center text-[11px]">
              <Star size={11} fill="currentColor" /> {p.rating}
            </span>
          ) : null}
          {p.has_location && <MapPin size={12} />}
          {p.edited && <PencilLine size={12} />}
        </div>
      </div>
    </div>
  );
}

function localStorageGet(k: string): string | null {
  try {
    return localStorage.getItem(`photolake.${k}`);
  } catch {
    return null;
  }
}
function localStorageSet(k: string, v: string) {
  try {
    localStorage.setItem(`photolake.${k}`, v);
  } catch {
    /* storage unavailable */
  }
}
