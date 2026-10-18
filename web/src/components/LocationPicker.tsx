import { maplibregl } from "../lib/maplibre";
import "maplibre-gl/dist/maplibre-gl.css";
import { Search } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Input } from "./ui";

export const MAP_STYLE = "https://tiles.openfreemap.org/styles/liberty";

/** Click or drag to place a pin. Includes place search (OpenStreetMap Nominatim). */
export function LocationPicker({
  lat,
  lon,
  onChange,
  height = 260,
}: {
  lat?: number | null;
  lon?: number | null;
  onChange: (lat: number, lon: number, label?: string) => void;
  height?: number;
}) {
  const el = useRef<HTMLDivElement>(null);
  const map = useRef<maplibregl.Map | null>(null);
  const marker = useRef<maplibregl.Marker | null>(null);
  const cb = useRef(onChange);
  cb.current = onChange;
  const [q, setQ] = useState("");
  const [results, setResults] = useState<{ display_name: string; lat: string; lon: string }[]>([]);

  useEffect(() => {
    if (!el.current) return;
    const has = lat != null && lon != null;
    const m = new maplibregl.Map({
      container: el.current,
      style: MAP_STYLE,
      center: has ? [lon!, lat!] : [-96, 39],
      zoom: has ? 12 : 2.5,
      attributionControl: { compact: true },
    });
    m.addControl(new maplibregl.NavigationControl({ showCompass: false }), "top-right");
    const mk = new maplibregl.Marker({ draggable: true, color: "#d95926" });
    if (has) mk.setLngLat([lon!, lat!]).addTo(m);
    mk.on("dragend", () => {
      const p = mk.getLngLat();
      cb.current(p.lat, p.lng);
    });
    m.on("click", (e) => {
      mk.setLngLat(e.lngLat).addTo(m);
      cb.current(e.lngLat.lat, e.lngLat.lng);
    });
    map.current = m;
    marker.current = mk;
    return () => m.remove();
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (lat == null || lon == null || !map.current || !marker.current) return;
    marker.current.setLngLat([lon, lat]).addTo(map.current);
  }, [lat, lon]);

  const search = async () => {
    if (!q.trim()) return;
    const r = await fetch(`https://nominatim.openstreetmap.org/search?format=json&limit=5&q=${encodeURIComponent(q)}`);
    setResults(r.ok ? await r.json() : []);
  };

  return (
    <div className="space-y-2">
      <form
        onSubmit={(e) => {
          e.preventDefault();
          search();
        }}
        className="relative"
      >
        <Search size={15} className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-muted" />
        <Input value={q} onChange={(e) => setQ(e.target.value)} placeholder="Search a place, or click the map" className="pl-8" />
        {results.length > 0 && (
          <div className="absolute z-20 mt-1 w-full overflow-hidden rounded-lg border border-line bg-s1 shadow-xl">
            {results.map((r) => (
              <button
                type="button"
                key={r.display_name}
                className="block w-full truncate px-3 py-2 text-left text-sm hover:bg-s3"
                onClick={() => {
                  const la = Number(r.lat), lo = Number(r.lon);
                  map.current?.flyTo({ center: [lo, la], zoom: 13 });
                  onChange(la, lo, r.display_name.split(",").slice(0, 3).join(",").trim());
                  setResults([]);
                }}
              >
                {r.display_name}
              </button>
            ))}
          </div>
        )}
      </form>
      <div ref={el} className="overflow-hidden rounded-lg border border-line" style={{ height }} />
    </div>
  );
}
