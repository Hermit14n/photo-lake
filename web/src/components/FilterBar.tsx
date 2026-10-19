import * as Dialog from "@radix-ui/react-dialog";
import { useQuery } from "@tanstack/react-query";
import { Bird, Image as ImageIcon, Images, RotateCcw, SlidersHorizontal, X } from "lucide-react";
import { useState } from "react";
import { api, toQuery, type Facets, type Filters } from "../lib/api";
import { Button, Combobox, SearchInput, Segmented, Select } from "./ui";

const MONTHS = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

export function useFacets(filters: Filters) {
  return useQuery({
    queryKey: ["facets", filters],
    queryFn: () => api.get<Facets>(`/facets${toQuery(filters)}`),
    placeholderData: (prev) => prev,
  });
}

/** One row of dropdowns shared by Library, Map and Analytics. Facet counts honor the other filters. */
export function FilterBar({
  filters,
  onChange,
  showSort = false,
  extra,
}: {
  filters: Filters;
  onChange: (changes: Partial<Record<keyof Filters, string | undefined>>) => void;
  showSort?: boolean;
  extra?: React.ReactNode;
}) {
  // Each dropdown's options are computed without its own filter, so you can switch values.
  const without = (k: keyof Filters) => ({ ...filters, [k]: undefined, sort: undefined });
  const species = useFacets(without("species")).data?.species ?? [];
  const cams = useFacets(without("camera")).data?.cameras ?? [];
  const lenses = useFacets(without("lens")).data?.lenses ?? [];
  const places = useFacets(without("place")).data?.places ?? [];
  const tags = useFacets(without("tag")).data?.tags ?? [];
  const years = useFacets(without("year")).data?.years ?? [];

  const [sheet, setSheet] = useState(false);
  const active = Object.entries(filters).some(([k, v]) => k !== "sort" && v);
  // Filters hidden behind the phone sheet (category and search stay visible).
  const inSheet = Object.entries(filters).filter(([k, v]) => !["sort", "category", "q"].includes(k) && v).length;
  const cat = (filters.category ?? "all") as "all" | "bird" | "general";
  const flags = filters.undated === "true" ? "undated" : filters.has_location === "false" ? "no_location" : filters.edited === "true" ? "edited" : undefined;
  const opt = (xs: { value: string | number; n: number; label?: string }[]) =>
    xs.map((x) => ({ value: String(x.value), label: x.label ?? String(x.value), count: x.n }));

  const dropdowns = (
    <>
      <Combobox
        value={filters.species}
        valueLabel={species.find((s) => s.value === filters.species)?.label}
        placeholder="Species"
        options={opt(species)}
        onChange={(v) => onChange({ species: v, category: v ? "bird" : filters.category })}
        emptyText="No species identified yet"
      />
      <Select value={filters.year} onChange={(v) => onChange({ year: v, from: undefined, to: undefined })} placeholder="Year" options={opt(years)} />
      <Select
        value={filters.month}
        onChange={(v) => onChange({ month: v })}
        placeholder="Month"
        options={MONTHS.map((m, i) => ({ value: String(i + 1), label: m }))}
      />
      <Select value={filters.camera} onChange={(v) => onChange({ camera: v })} placeholder="Camera" options={opt(cams)} />
      <Select value={filters.lens} onChange={(v) => onChange({ lens: v })} placeholder="Lens" options={opt(lenses)} />
      <Select value={filters.place} onChange={(v) => onChange({ place: v })} placeholder="Place" options={opt(places)} />
      <Select value={filters.tag} onChange={(v) => onChange({ tag: v })} placeholder="Tag" options={opt(tags)} />
      <Select
        value={flags}
        placeholder="Status"
        onChange={(v) =>
          onChange({
            undated: v === "undated" ? "true" : undefined,
            has_location: v === "no_location" ? "false" : undefined,
            edited: v === "edited" ? "true" : undefined,
          })
        }
        options={[
          { value: "undated", label: "No date" },
          { value: "no_location", label: "No location" },
          { value: "edited", label: "Edited by me" },
        ]}
      />
      <Select
        value={filters.rating_min}
        onChange={(v) => onChange({ rating_min: v })}
        placeholder="Rating"
        options={[1, 2, 3, 4, 5].map((r) => ({ value: String(r), label: `${"★".repeat(r)} & up` }))}
      />
      {showSort && (
        <Select
          value={filters.sort}
          allowAny={false}
          onChange={(v) => onChange({ sort: v === "captured_desc" ? undefined : v })}
          placeholder="Newest first"
          options={[
            { value: "captured_desc", label: "Newest first" },
            { value: "captured_asc", label: "Oldest first" },
            { value: "imported_desc", label: "Recently imported" },
            { value: "rating_desc", label: "Highest rated" },
          ]}
        />
      )}
    </>
  );
  const clear = active && (
    <Button
      variant="ghost"
      onClick={() =>
        onChange(Object.fromEntries(Object.keys(filters).filter((k) => k !== "sort").map((k) => [k, undefined])) as Partial<Filters>)
      }
    >
      <RotateCcw size={14} /> Clear
    </Button>
  );

  return (
    <div className="flex flex-wrap items-center gap-2">
      <Segmented
        value={cat}
        onChange={(v) => onChange({ category: v === "all" ? undefined : v })}
        options={[
          { value: "all", label: <><Images size={14} /> All</> },
          { value: "bird", label: <><Bird size={14} /> Birds</> },
          { value: "general", label: <><ImageIcon size={14} /> General</> },
        ]}
      />
      <Dialog.Root open={sheet} onOpenChange={setSheet}>
        <Dialog.Trigger asChild>
          <Button className="ml-auto md:hidden" aria-label="Filters">
            <SlidersHorizontal size={14} /> Filters
            {inSheet > 0 && <span className="rounded-full bg-accent px-1.5 text-[11px] leading-4 text-white">{inSheet}</span>}
          </Button>
        </Dialog.Trigger>
        <Dialog.Portal>
          <Dialog.Overlay className="fixed inset-0 z-40 bg-black/60 md:hidden" />
          <Dialog.Content className="fixed inset-x-0 bottom-0 z-50 max-h-[85dvh] overflow-y-auto rounded-t-2xl border-t border-line bg-s1 p-4 pb-[max(1rem,env(safe-area-inset-bottom))] shadow-2xl md:hidden">
            <div className="mb-3 flex items-center justify-between">
              <Dialog.Title className="text-base font-semibold text-ink">Filters</Dialog.Title>
              <Dialog.Close className="rounded-lg p-1.5 text-ink2 hover:bg-s2" aria-label="Close"><X size={18} /></Dialog.Close>
            </div>
            <Dialog.Description className="sr-only">Narrow down the photos shown</Dialog.Description>
            <div className="grid grid-cols-1 gap-2 [&>*]:w-full">
              {dropdowns}
            </div>
            <div className="mt-4 flex gap-2">
              {clear}
              <Button variant="primary" className="ml-auto" onClick={() => setSheet(false)}>Done</Button>
            </div>
          </Dialog.Content>
        </Dialog.Portal>
      </Dialog.Root>
      <div className="hidden md:contents">
        {dropdowns}
      </div>
      <SearchInput value={filters.q ?? ""} onChange={(q) => onChange({ q: q || undefined })} placeholder="Search notes, species, places…" />
      <div className="hidden md:contents">
        {clear}
      </div>
      {extra}
    </div>
  );
}
