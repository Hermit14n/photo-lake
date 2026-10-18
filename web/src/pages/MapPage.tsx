import { useQuery } from "@tanstack/react-query";
import { maplibregl } from "../lib/maplibre";
import type { GeoJSONSource } from "maplibre-gl";
import "maplibre-gl/dist/maplibre-gl.css";
import { MapPinOff } from "lucide-react";
import { useEffect, useRef } from "react";
import { FilterBar } from "../components/FilterBar";
import { MAP_STYLE } from "../components/LocationPicker";
import { useTheme } from "../components/Chart";
import { api, fmtDate, img, toQuery, type Filters } from "../lib/api";
import { navigate, setParams, type Route } from "../lib/route";

type Point = { photo_id: string; lat: number; lon: number; category: string; species: string | null; captured_local: string | null };

export function MapPage({ route, filters }: { route: Route; filters: Filters }) {
  const t = useTheme();
  const el = useRef<HTMLDivElement>(null);
  const map = useRef<maplibregl.Map | null>(null);
  const ready = useRef(false);
  const fitted = useRef(false);
  const applyRef = useRef<(() => void) | null>(null);
  const q = useQuery({ queryKey: ["map", filters], queryFn: () => api.get<Point[]>(`/map${toQuery(filters)}`) });

  useEffect(() => {
    if (!el.current) return;
    const m = new maplibregl.Map({ container: el.current, style: MAP_STYLE, center: [-96, 39], zoom: 3, attributionControl: { compact: true } });
    m.addControl(new maplibregl.NavigationControl(), "top-right");
    m.on("load", () => {
      m.addSource("photos", { type: "geojson", data: { type: "FeatureCollection", features: [] }, cluster: true, clusterRadius: 44, clusterMaxZoom: 15 });
      m.addLayer({ id: "clusters", type: "circle", source: "photos", filter: ["has", "point_count"], paint: { "circle-color": t.bird, "circle-opacity": 0.85, "circle-stroke-color": "#ffffff", "circle-stroke-width": 2, "circle-radius": ["step", ["get", "point_count"], 14, 10, 18, 50, 24, 200, 30] } });
      m.addLayer({ id: "cluster-count", type: "symbol", source: "photos", filter: ["has", "point_count"], layout: { "text-field": "{point_count_abbreviated}", "text-size": 12, "text-font": ["Noto Sans Regular"] }, paint: { "text-color": "#ffffff" } });
      m.addLayer({ id: "point", type: "circle", source: "photos", filter: ["!", ["has", "point_count"]], paint: { "circle-color": ["match", ["get", "category"], "bird", t.bird, t.general], "circle-radius": 7, "circle-stroke-color": "#ffffff", "circle-stroke-width": 2 } });
      m.on("click", "clusters", async (e) => {
        const f = e.features?.[0];
        if (!f) return;
        const src = m.getSource("photos") as GeoJSONSource;
        const zoom = await src.getClusterExpansionZoom(f.properties.cluster_id);
        m.easeTo({ center: (f.geometry as GeoJSON.Point).coordinates as [number, number], zoom });
      });
      m.on("click", "point", (e) => {
        const f = e.features?.[0];
        if (!f) return;
        const p = f.properties as Point;
        const div = document.createElement("div");
        div.className = "w-48 cursor-pointer";
        // Built with DOM APIs, never innerHTML: species names and places are user-editable.
        const thumb = document.createElement("img");
        thumb.src = img(p.photo_id, "thumb");
        thumb.className = "h-32 w-48 rounded-md object-cover";
        const label = document.createElement("div");
        label.className = "mt-1 text-xs font-medium";
        label.textContent = p.species ?? (p.category === "bird" ? "Bird" : "Photo");
        const when = document.createElement("div");
        when.className = "text-[11px] opacity-70";
        when.textContent = fmtDate(p.captured_local);
        div.append(thumb, label, when);
        div.onclick = () => navigate("map", Object.fromEntries([...new URLSearchParams(window.location.hash.split("?")[1] ?? ""), ["photo", p.photo_id]]));
        new maplibregl.Popup({ offset: 10, closeButton: false }).setLngLat((f.geometry as GeoJSON.Point).coordinates as [number, number]).setDOMContent(div).addTo(m);
      });
      for (const l of ["clusters", "point"]) {
        m.on("mouseenter", l, () => (m.getCanvas().style.cursor = "pointer"));
        m.on("mouseleave", l, () => (m.getCanvas().style.cursor = ""));
      }
      ready.current = true;
      applyRef.current?.();
    });
    map.current = m;
    return () => m.remove();
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    const m = map.current;
    const pts = q.data;
    if (!m || !pts) return;
    const apply = () => {
      (m.getSource("photos") as GeoJSONSource | undefined)?.setData({
        type: "FeatureCollection",
        features: pts.map((p) => ({ type: "Feature", geometry: { type: "Point", coordinates: [p.lon, p.lat] }, properties: p })),
      });
      if (pts.length && !fitted.current) {
        const b = new maplibregl.LngLatBounds();
        pts.forEach((p) => b.extend([p.lon, p.lat]));
        m.fitBounds(b, { padding: 60, maxZoom: 13, duration: 0 });
        fitted.current = true;
      }
    };
    applyRef.current = apply;
    if (ready.current) apply();
  }, [q.data]);

  return (
    <div className="flex h-full flex-col">
      <div className="border-b border-line px-4 py-3">
        <FilterBar filters={filters} onChange={(c) => { fitted.current = false; setParams(route, c); }} extra={<span className="ml-auto text-sm text-ink2">{q.data ? `${q.data.length.toLocaleString()} located photos` : ""}</span>} />
      </div>
      <div className="relative flex-1">
        <div ref={el} style={{ position: "absolute", inset: 0 }} />
        {q.data && q.data.length === 0 && (
          <div className="absolute left-1/2 top-6 -translate-x-1/2 rounded-xl border border-line bg-s1/95 px-4 py-3 text-sm text-ink2 shadow-xl">
            <div className="flex items-center gap-2 font-medium text-ink"><MapPinOff size={16} /> No located photos for these filters</div>
            Your camera doesn't record GPS. Set locations from a photo's detail panel, select many in the Library and use <b>Location</b>, or import a GPX track.
          </div>
        )}
      </div>
    </div>
  );
}
