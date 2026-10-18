import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import clsx from "clsx";
import {
  Bird, Check, ChevronLeft, ChevronRight, Download, Eye, EyeOff, History, Maximize2, Pencil, Plus, RotateCcw, Square, Star, Trash2, X,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { api, fmtDate, img, toQuery, useReadOnly, type Bird as BirdBox, type Candidate, type Edit, type Filters, type PhotoDetail as Detail, type SpeciesHit } from "../lib/api";
import { LocationPicker } from "./LocationPicker";
import { Button, Combobox, Input, Spinner } from "./ui";

const UNCERTAIN = 0.3;

export function PhotoDetail({ id, filters, onClose, onNavigate }: { id: string; filters: Filters; onClose: () => void; onNavigate: (id: string) => void }) {
  const qc = useQueryClient();
  const readOnly = useReadOnly();
  const detail = useQuery({ queryKey: ["photo", id], queryFn: () => api.get<Detail>(`/photos/${id}`) });
  const ids = useQuery({ queryKey: ["ids", filters], queryFn: () => api.get<string[]>(`/photos/ids${toQuery(filters)}`), staleTime: 30_000 });
  const pos = ids.data?.indexOf(id) ?? -1;
  const prev = pos > 0 ? ids.data![pos - 1] : undefined;
  const next = pos >= 0 && pos < (ids.data?.length ?? 0) - 1 ? ids.data![pos + 1] : undefined;

  const save = useMutation({
    mutationFn: (edits: Edit[]) => api.post(`/photos/${id}/edits`, { edits }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["photo", id] });
      qc.invalidateQueries({ queryKey: ["photos"] });
      qc.invalidateQueries({ queryKey: ["facets"] });
      qc.invalidateQueries({ queryKey: ["analytics"] });
    },
  });
  const edit = useCallback((e: Edit | Edit[]) => save.mutate(Array.isArray(e) ? e : [e]), [save]);

  const [showBoxes, setShowBoxes] = useState(true);
  const [drawing, setDrawing] = useState(false);
  const [full, setFull] = useState(false);
  const [tab, setTab] = useState<"info" | "history">("info");

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.target as HTMLElement)?.closest("input, textarea, [role=dialog] [contenteditable]")) return;
      if (e.key === "Escape") drawing ? setDrawing(false) : onClose();
      else if (e.key === "ArrowLeft" && prev) onNavigate(prev);
      else if (e.key === "ArrowRight" && next) onNavigate(next);
      else if (/^[0-5]$/.test(e.key) && !readOnly) edit({ field: "rating", value: Number(e.key) });
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [prev, next, drawing, onClose, onNavigate, edit, readOnly]);

  // Prefetch neighbours for instant ←/→.
  useEffect(() => {
    for (const n of [prev, next]) {
      if (!n) continue;
      new Image().src = img(n, "preview");
      qc.prefetchQuery({ queryKey: ["photo", n], queryFn: () => api.get<Detail>(`/photos/${n}`) });
    }
  }, [prev, next, qc]);

  const d = detail.data;
  const p = d?.photo;
  const nextBox = useMemo(() => (d ? Math.max(-1, ...d.birds.map((b) => b.box_index), ...d.candidates.map((c) => c.box_index)) + 1 : 0), [d]);

  return (
    <div className="fixed inset-0 z-30 flex bg-bg/95 backdrop-blur-sm" role="dialog" aria-label="Photo detail">
      <div className="relative flex min-w-0 flex-1 flex-col">
        <div className="flex items-center gap-2 px-4 py-2.5">
          <Button variant="ghost" onClick={onClose}><X size={16} /> Close</Button>
          <div className="min-w-0 flex-1 truncate text-sm text-ink2">
            {p ? <><span className="text-ink">{p.title || p.original_filename}</span> · {fmtDate(p.captured_at, { dateStyle: "full", timeStyle: "short" })}</> : "…"}
            {pos >= 0 && <span className="ml-2 text-muted">{pos + 1} / {ids.data!.length}</span>}
          </div>
          <Button variant="ghost" size="sm" onClick={() => setShowBoxes((s) => !s)} title="Toggle bird boxes">{showBoxes ? <Eye size={15} /> : <EyeOff size={15} />} Boxes</Button>
          {!readOnly && <Button variant={drawing ? "primary" : "ghost"} size="sm" onClick={() => setDrawing((v) => !v)} title="Draw a box around a bird the detector missed"><Square size={15} /> {drawing ? "Drag on the photo…" : "Add bird"}</Button>}
          <Button variant="ghost" size="sm" onClick={() => setFull((f) => !f)} title="Load full resolution"><Maximize2 size={15} /> {full ? "Preview" : "Full res"}</Button>
          <a href={img(id, "original")} className="inline-flex h-7 items-center gap-1 rounded-lg px-2.5 text-xs text-ink2 hover:bg-s2 hover:text-ink"><Download size={14} /> Original</a>
        </div>
        <div className="relative flex min-h-0 flex-1 items-center justify-center px-14 pb-4">
          {prev && <NavButton side="left" onClick={() => onNavigate(prev)} />}
          {next && <NavButton side="right" onClick={() => onNavigate(next)} />}
          {d ? (
            <ImageWithBoxes
              key={id}
              src={img(id, full ? "full" : "preview")}
              birds={showBoxes ? d.birds : []}
              drawing={drawing}
              onDrawn={(bbox) => {
                setDrawing(false);
                edit({ target: `box:${nextBox}`, field: "bbox", value: bbox });
              }}
            />
          ) : (
            <Spinner />
          )}
        </div>
      </div>

      <aside className="flex w-[400px] shrink-0 flex-col border-l border-line bg-s1">
        <div className="flex border-b border-line px-2">
          {(["info", "history"] as const).map((t) => (
            <button key={t} onClick={() => setTab(t)} className={clsx("flex items-center gap-1.5 border-b-2 px-3 py-3 text-sm", tab === t ? "border-accent text-ink" : "border-transparent text-ink2 hover:text-ink")}>
              {t === "info" ? <><Pencil size={14} /> Details</> : <><History size={14} /> History {d?.history.length ? `(${d.history.length})` : ""}</>}
            </button>
          ))}
          <div className="ml-auto flex items-center pr-2 text-xs text-muted">{save.isPending ? "Saving…" : save.isError ? <span className="text-danger">{String(save.error)}</span> : save.isSuccess ? "Saved to lake" : ""}</div>
        </div>
        <div className="flex-1 overflow-y-auto p-4">
          {/* A disabled fieldset turns every editor inside into read-only controls for viewers. */}
          <fieldset disabled={readOnly} className="min-w-0">
            {!d ? <Spinner /> : tab === "info" ? <InfoPanel d={d} edit={edit} /> : <HistoryPanel d={d} />}
          </fieldset>
        </div>
      </aside>
    </div>
  );
}

function NavButton({ side, onClick }: { side: "left" | "right"; onClick: () => void }) {
  return (
    <button onClick={onClick} aria-label={side === "left" ? "Previous photo" : "Next photo"} className={clsx("absolute top-1/2 z-10 -translate-y-1/2 rounded-full bg-s2/80 p-2 text-ink hover:bg-s3", side === "left" ? "left-2" : "right-2")}>
      {side === "left" ? <ChevronLeft size={22} /> : <ChevronRight size={22} />}
    </button>
  );
}

function ImageWithBoxes({ src, birds, drawing, onDrawn }: { src: string; birds: BirdBox[]; drawing: boolean; onDrawn: (b: [number, number, number, number]) => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const [drag, setDrag] = useState<{ x0: number; y0: number; x1: number; y1: number } | null>(null);
  const [loaded, setLoaded] = useState(false);
  const rel = (e: React.PointerEvent) => {
    const r = ref.current!.getBoundingClientRect();
    return { x: Math.min(1, Math.max(0, (e.clientX - r.left) / r.width)), y: Math.min(1, Math.max(0, (e.clientY - r.top) / r.height)) };
  };
  const box = drag && { x: Math.min(drag.x0, drag.x1), y: Math.min(drag.y0, drag.y1), w: Math.abs(drag.x1 - drag.x0), h: Math.abs(drag.y1 - drag.y0) };
  return (
    <div
      ref={ref}
      className={clsx("relative max-h-full max-w-full", drawing && "cursor-crosshair")}
      onPointerDown={(e) => {
        if (!drawing) return;
        const p = rel(e);
        (e.target as Element).setPointerCapture(e.pointerId);
        setDrag({ x0: p.x, y0: p.y, x1: p.x, y1: p.y });
      }}
      onPointerMove={(e) => drag && setDrag({ ...drag, x1: rel(e).x, y1: rel(e).y })}
      onPointerUp={() => {
        if (box && box.w > 0.01 && box.h > 0.01) onDrawn([box.x, box.y, box.w, box.h]);
        setDrag(null);
      }}
    >
      <img src={src} alt="" draggable={false} onLoad={() => setLoaded(true)} className="block max-h-[calc(100vh-80px)] max-w-full select-none rounded-lg object-contain" />
      {!loaded && <div className="absolute inset-0 flex items-center justify-center"><Spinner /></div>}
      {loaded &&
        birds.map((b) => {
          const uncertain = b.species_source === "model" && (b.final_score ?? 0) < UNCERTAIN;
          return (
            <div key={b.box_index} className="pointer-events-none absolute rounded-md border-2" style={{ left: `${b.bbox_x * 100}%`, top: `${b.bbox_y * 100}%`, width: `${b.bbox_w * 100}%`, height: `${b.bbox_h * 100}%`, borderColor: b.confirmed ? "#0ca30c" : "#d95926", borderStyle: uncertain ? "dashed" : "solid" }}>
              <span className={clsx("absolute left-0 whitespace-nowrap rounded bg-black/70 px-1.5 py-0.5 text-xs text-white", b.box_index % 2 ? "-bottom-6" : "-top-6")}>
                {b.common_name ?? "Unidentified bird"}
                {b.species_source === "model" && b.final_score != null && ` · ${Math.round(b.final_score * 100)}%`}
                {b.confirmed && " ✓"}
              </span>
            </div>
          );
        })}
      {box && <div className="pointer-events-none absolute border-2 border-dashed border-focus bg-focus/10" style={{ left: `${box.x * 100}%`, top: `${box.y * 100}%`, width: `${box.w * 100}%`, height: `${box.h * 100}%` }} />}
    </div>
  );
}

function Section({ title, children, action }: { title: string; children: ReactNode; action?: ReactNode }) {
  return (
    <section className="mb-6">
      <div className="mb-2 flex items-center justify-between">
        <h4 className="text-xs font-semibold uppercase tracking-wide text-muted">{title}</h4>
        {action}
      </div>
      {children}
    </section>
  );
}

function InfoPanel({ d, edit }: { d: Detail; edit: (e: Edit | Edit[]) => void }) {
  const p = d.photo;
  const o = d.original ?? {};
  const edited = new Set<string>((p.edited_fields ?? "").split(",").filter(Boolean));
  const [locEdit, setLocEdit] = useState(false);
  const tags: string[] = p.tags ? String(p.tags).split(",") : [];
  const field = (name: string, label: string, type: "text" | "number" | "datetime" = "text", fmt?: (v: any) => string) => (
    <EditableField key={name} label={label} value={p[name]} original={o[name]} edited={edited.has(name)} type={type} fmt={fmt}
      onSave={(v) => edit(v === null ? { field: name, op: "clear" } : { field: name, value: v })}
      onRevert={() => edit({ field: name, op: "revert" })} />
  );
  return (
    <>
      <Section title="Rating & category">
        <div className="flex items-center justify-between">
          <div className="flex">
            {[1, 2, 3, 4, 5].map((r) => (
              <button key={r} onClick={() => edit({ field: "rating", value: p.rating === r ? 0 : r })} aria-label={`${r} stars`} className={clsx("p-0.5", (p.rating ?? 0) >= r ? "text-[#eda100]" : "text-muted hover:text-ink2")}>
                <Star size={18} fill={(p.rating ?? 0) >= r ? "currentColor" : "none"} />
              </button>
            ))}
          </div>
          <div className="flex rounded-lg border border-line bg-s2 p-0.5">
            {(["bird", "general"] as const).map((c) => (
              <button key={c} onClick={() => edit({ field: "category", value: c })} className={clsx("rounded-md px-3 py-1 text-sm capitalize", p.category === c ? "bg-s3 text-ink" : "text-ink2")}>{c}</button>
            ))}
          </div>
        </div>
      </Section>

      <BirdsSection d={d} edit={edit} />

      <Section title="Notes">
        <EditableField label="Title" value={p.title} edited={edited.has("title")} onSave={(v) => edit(v === null ? { field: "title", op: "clear" } : { field: "title", value: v })} onRevert={() => edit({ field: "title", op: "revert" })} />
        <NoteField value={p.note ?? ""} onSave={(v) => edit(v ? { field: "note", value: v } : { field: "note", op: "clear" })} />
        <TagEditor tags={tags} onChange={(t) => edit(t.length ? { field: "tags", value: t } : { field: "tags", op: "clear" })} />
      </Section>

      <Section title="Location" action={<Button size="sm" variant="ghost" onClick={() => setLocEdit((v) => !v)}>{locEdit ? "Done" : p.gps_lat != null ? "Change" : "Set location"}</Button>}>
        {locEdit && (
          <div className="mb-3">
            <LocationPicker lat={p.gps_lat} lon={p.gps_lon} onChange={(lat, lon, label) => {
              const e: Edit[] = [{ field: "gps_lat", value: lat }, { field: "gps_lon", value: lon }];
              if (label) e.push({ field: "place_name", value: label });
              edit(e);
            }} />
          </div>
        )}
        {field("place_name", "Place")}
        {field("gps_lat", "Latitude", "number", (v) => Number(v).toFixed(5))}
        {field("gps_lon", "Longitude", "number", (v) => Number(v).toFixed(5))}
        {field("gps_alt", "Altitude (m)", "number")}
      </Section>

      <Section title="Capture">
        {field("captured_at", "Date & time", "datetime", (v) => fmtDate(v))}
        {field("camera_make", "Make")}
        {field("camera_model", "Camera")}
        {field("lens", "Lens")}
        {field("focal_mm", "Focal length (mm)", "number")}
        {field("aperture", "Aperture (f/)", "number")}
        {field("shutter_s", "Shutter (s)", "number", (v) => (v < 1 ? `1/${Math.round(1 / v)}` : `${v}`))}
        {field("iso", "ISO", "number")}
        {field("width", "Width", "number")}
        {field("height", "Height", "number")}
      </Section>

      <CustomFields custom={d.custom} edit={edit} />

      <Section title="File">
        <ReadOnly label="Filename" value={p.original_filename} />
        <ReadOnly label="Imported" value={fmtDate(p.imported_at)} />
        <ReadOnly label="Size" value={`${(Number(p.file_size) / 1e6).toFixed(1)} MB${p.raw_key ? " (RAW + JPEG)" : ""}`} />
        <ReadOnly label="SHA-256" value={String(p.sha256).slice(0, 16) + "…"} />
      </Section>
    </>
  );
}

function BirdsSection({ d, edit }: { d: Detail; edit: (e: Edit | Edit[]) => void }) {
  const byBox = useMemo(() => {
    const m = new Map<number, Candidate[]>();
    for (const c of d.candidates) m.set(c.box_index, [...(m.get(c.box_index) ?? []), c]);
    return m;
  }, [d.candidates]);
  if (d.birds.length === 0)
    return (
      <Section title="Birds">
        <p className="text-sm text-ink2">No birds detected. Use <span className="text-ink">Add bird</span> above to draw a box around one.</p>
      </Section>
    );
  return (
    <Section title={`Birds (${d.birds.length})`}>
      <div className="space-y-3">
        {d.birds.map((b) => (
          <BirdCard key={b.box_index} b={b} candidates={byBox.get(b.box_index) ?? []} edit={edit} />
        ))}
      </div>
    </Section>
  );
}

function BirdCard({ b, candidates, edit }: { b: BirdBox; candidates: Candidate[]; edit: (e: Edit | Edit[]) => void }) {
  const target = `box:${b.box_index}`;
  const [hits, setHits] = useState<SpeciesHit[]>([]);
  const search = async (q: string) => setHits(q.trim().length >= 2 ? await api.get<SpeciesHit[]>(`/species?q=${encodeURIComponent(q)}`) : []);
  const uncertain = b.species_source === "model" && (b.final_score ?? 0) < UNCERTAIN;
  const setSpecies = (code: string) => edit({ target, field: "species_code", value: code });
  return (
    <div className="rounded-lg border border-line bg-s2 p-3">
      <div className="flex items-start justify-between gap-2">
        <div className="min-w-0">
          <div className="flex items-center gap-1.5 font-medium text-ink">
            <Bird size={15} className="shrink-0 text-accent" />
            <span className="truncate">{b.common_name ?? "Unidentified bird"}</span>
          </div>
          {b.scientific_name && <div className="text-xs italic text-ink2">{b.scientific_name}</div>}
          <div className="mt-1 text-xs text-muted">
            {b.species_source === "user" ? "Set by you" : b.species_source === "model" ? `Model guess · ${Math.round((b.final_score ?? 0) * 100)}%${uncertain ? " · uncertain" : ""}` : b.user_added ? "Box drawn by you" : ""}
            {b.confirmed && <span className="ml-1 text-[#0ca30c]">· confirmed</span>}
          </div>
        </div>
        <div className="flex shrink-0 gap-1">
          {!b.confirmed && b.species_code && <Button size="sm" onClick={() => edit({ target, field: "confirmed", value: true })} title="Confirm this species"><Check size={13} /> Confirm</Button>}
          <Button size="sm" variant="ghost" title="Remove this box" onClick={() => edit({ target, field: "removed", value: true })}><Trash2 size={13} /></Button>
        </div>
      </div>
      {candidates.length > 0 && (
        <div className="mt-2 space-y-1">
          {candidates.map((c) => (
            <button key={c.rank} onClick={() => setSpecies(c.species_code)} className={clsx("flex w-full items-center gap-2 rounded-md px-2 py-1 text-left text-sm hover:bg-s3", c.species_code === b.species_code && "bg-s3")}>
              <span className="w-4 text-xs text-muted">{c.rank}</span>
              <span className="min-w-0 flex-1 truncate text-ink">{c.common_name}</span>
              <span className="h-1.5 w-16 overflow-hidden rounded-full bg-s3" aria-hidden><span className="block h-full rounded-full bg-accent" style={{ width: `${Math.max(2, c.final_score * 100)}%` }} /></span>
              <span className="w-9 text-right text-xs tabular-nums text-ink2">{Math.round(c.final_score * 100)}%</span>
            </button>
          ))}
        </div>
      )}
      <div className="mt-2 flex items-center gap-2">
        <Combobox
          placeholder="Species"
          onSearch={search}
          options={hits.map((h) => ({ value: h.code, label: `${h.common} — ${h.sci}` }))}
          onChange={(v) => v && setSpecies(v)}
          emptyText="Type at least 2 letters"
          trigger={<button className="inline-flex h-7 items-center gap-1 rounded-lg border border-line px-2.5 text-xs text-ink2 hover:bg-s3 hover:text-ink"><Pencil size={12} /> Choose another species</button>}
        />
        {b.species_source === "user" && (
          <button className="text-xs text-ink2 hover:text-ink" onClick={() => edit([{ target, field: "species_code", op: "revert" }, { target, field: "confirmed", op: "revert" }])}><RotateCcw size={11} className="inline" /> Use model guess</button>
        )}
      </div>
    </div>
  );
}

function EditableField({ label, value, original, edited, type = "text", fmt, onSave, onRevert }: {
  label: string; value: any; original?: any; edited?: boolean; type?: "text" | "number" | "datetime"; fmt?: (v: any) => string;
  onSave: (v: string | number | null) => void; onRevert: () => void;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");
  const shown = value === null || value === undefined || value === "" ? null : fmt ? fmt(value) : String(value);
  const start = () => {
    setDraft(value == null ? "" : type === "datetime" ? toLocalInput(value) : String(value));
    setEditing(true);
  };
  const commit = () => {
    setEditing(false);
    const v = draft.trim();
    if (v === "") return value != null && onSave(null);
    if (type === "datetime") return onSave(new Date(v).toISOString());
    if (type === "number") return onSave(Number(v));
    onSave(v);
  };
  return (
    <div className="group flex min-h-8 items-center gap-2 py-0.5 text-sm">
      <div className="w-32 shrink-0 text-ink2">{label}</div>
      {editing ? (
        <Input autoFocus type={type === "datetime" ? "datetime-local" : type === "number" ? "number" : "text"} step="any" value={draft} onChange={(e) => setDraft(e.target.value)} onBlur={commit}
          onKeyDown={(e) => { if (e.key === "Enter") commit(); if (e.key === "Escape") setEditing(false); }} className="h-7" />
      ) : (
        <button onClick={start} className="flex min-w-0 flex-1 items-center gap-1.5 rounded-md px-1.5 py-1 text-left hover:bg-s2" title="Click to edit">
          <span className={clsx("truncate", shown ? "text-ink" : "text-muted")}>{shown ?? "—"}</span>
          {edited && <span className="shrink-0 rounded bg-focus/15 px-1 text-[10px] font-medium text-focus" title={`Original: ${original == null ? "—" : fmt ? fmt(original) : original}`}>edited</span>}
          <Pencil size={11} className="ml-auto shrink-0 text-muted opacity-0 group-hover:opacity-100" />
        </button>
      )}
      {edited && !editing && (
        <button onClick={onRevert} title={`Revert to original (${original == null ? "empty" : fmt ? fmt(original) : original})`} className="shrink-0 text-muted hover:text-ink"><RotateCcw size={13} /></button>
      )}
    </div>
  );
}

function toLocalInput(ts: string) {
  const d = new Date(String(ts).replace(" ", "T"));
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

function ReadOnly({ label, value }: { label: string; value: ReactNode }) {
  return (
    <div className="flex min-h-8 items-center gap-2 py-0.5 text-sm">
      <div className="w-32 shrink-0 text-ink2">{label}</div>
      <div className="truncate px-1.5 text-ink">{value}</div>
    </div>
  );
}

function NoteField({ value, onSave }: { value: string; onSave: (v: string) => void }) {
  const [draft, setDraft] = useState(value);
  useEffect(() => setDraft(value), [value]);
  return (
    <textarea value={draft} onChange={(e) => setDraft(e.target.value)} onBlur={() => draft.trim() !== value && onSave(draft.trim())} placeholder="Add a note… (saved when you click away)" rows={3}
      className="my-2 w-full resize-y rounded-lg border border-line bg-s2 p-2.5 text-sm text-ink placeholder:text-muted outline-none focus:border-focus" />
  );
}

function TagEditor({ tags, onChange }: { tags: string[]; onChange: (t: string[]) => void }) {
  const [draft, setDraft] = useState("");
  const add = () => {
    const t = draft.trim().toLowerCase();
    if (t && !tags.includes(t)) onChange([...tags, t]);
    setDraft("");
  };
  return (
    <div className="flex flex-wrap items-center gap-1.5">
      {tags.map((t) => (
        <span key={t} className="inline-flex h-7 items-center gap-1 rounded-full bg-s3 pl-2.5 pr-1 text-xs text-ink">
          {t}
          <button onClick={() => onChange(tags.filter((x) => x !== t))} aria-label={`Remove tag ${t}`} className="rounded-full p-0.5 text-muted hover:text-ink"><X size={12} /></button>
        </span>
      ))}
      <input value={draft} onChange={(e) => setDraft(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter" || e.key === ",") { e.preventDefault(); add(); } }} onBlur={add}
        placeholder="+ tag" className="h-7 w-24 rounded-full border border-dashed border-line bg-transparent px-2.5 text-xs text-ink outline-none placeholder:text-muted focus:border-focus" />
    </div>
  );
}

function CustomFields({ custom, edit }: { custom: Record<string, string>; edit: (e: Edit | Edit[]) => void }) {
  const [name, setName] = useState("");
  const [value, setValue] = useState("");
  const entries = Object.entries(custom);
  return (
    <Section title="Custom fields">
      {entries.map(([k, v]) => (
        <EditableField key={k} label={k} value={v} onSave={(nv) => edit(nv === null ? { field: `custom.${k}`, op: "clear" } : { field: `custom.${k}`, value: nv })} onRevert={() => edit({ field: `custom.${k}`, op: "clear" })} />
      ))}
      <form className="mt-1 flex gap-1.5" onSubmit={(e) => { e.preventDefault(); const k = name.trim().toLowerCase().replace(/\s+/g, "_"); if (k && value.trim()) { edit({ field: `custom.${k}`, value: value.trim() }); setName(""); setValue(""); } }}>
        <Input value={name} onChange={(e) => setName(e.target.value)} placeholder="Field (e.g. weather)" className="h-8" />
        <Input value={value} onChange={(e) => setValue(e.target.value)} placeholder="Value" className="h-8" />
        <Button size="sm" type="submit" className="h-8"><Plus size={13} /></Button>
      </form>
    </Section>
  );
}

function HistoryPanel({ d }: { d: Detail }) {
  if (!d.history.length) return <p className="text-sm text-ink2">No edits yet. Every change you make is stored as a separate event in the lake, so nothing is ever overwritten.</p>;
  return (
    <ol className="space-y-2">
      {d.history.map((h, i) => (
        <li key={i} className="rounded-lg border border-line bg-s2 p-2.5 text-sm">
          <div className="flex justify-between text-xs text-muted"><span>{h.target === "photo" ? "Photo" : `Bird ${Number(h.target.slice(4)) + 1}`}</span><span>{fmtDate(h.created_at)}</span></div>
          <div className="mt-0.5 text-ink">
            <span className="font-medium">{h.field.replace("custom.", "")}</span>{" "}
            {h.op === "set" ? <>→ <span className="text-ink2">{h.value}</span></> : <span className="text-ink2">{h.op === "clear" ? "cleared" : "reverted to original"}</span>}
          </div>
        </li>
      ))}
    </ol>
  );
}
