import { useQuery } from "@tanstack/react-query";
import { Bird, Image as ImageIcon, Images, RotateCcw } from "lucide-react";
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

  const active = Object.entries(filters).some(([k, v]) => k !== "sort" && v);
  const cat = (filters.category ?? "all") as "all" | "bird" | "general";
  const flags = filters.undated === "true" ? "undated" : filters.has_location === "false" ? "no_location" : filters.edited === "true" ? "edited" : undefined;
  const opt = (xs: { value: string | number; n: number; label?: string }[]) =>
    xs.map((x) => ({ value: String(x.value), label: x.label ?? String(x.value), count: x.n }));

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
      <SearchInput value={filters.q ?? ""} onChange={(q) => onChange({ q: q || undefined })} placeholder="Search notes, species, places…" />
      {active && (
        <Button
          variant="ghost"
          onClick={() =>
            onChange(Object.fromEntries(Object.keys(filters).filter((k) => k !== "sort").map((k) => [k, undefined])) as Partial<Filters>)
          }
        >
          <RotateCcw size={14} /> Clear
        </Button>
      )}
      {extra}
    </div>
  );
}
