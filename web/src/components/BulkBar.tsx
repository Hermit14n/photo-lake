import * as Dialog from "@radix-ui/react-dialog";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Bird, Clock, Image as ImageIcon, MapPin, Star, Tag, X } from "lucide-react";
import { useState, type ReactNode } from "react";
import { api, type Edit } from "../lib/api";
import { LocationPicker } from "./LocationPicker";
import { Button, Input } from "./ui";

type BulkBody = { photo_ids: string[]; edits?: Edit[]; time_shift_seconds?: number; add_tags?: string[]; remove_tags?: string[] };

export function useBulk(onDone?: () => void) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (b: BulkBody) => api.post<{ written: number }>("/edits/bulk", b),
    onSuccess: () => {
      qc.invalidateQueries();
      onDone?.();
    },
  });
}

export function BulkBar({ ids, onDone, onSelectAll }: { ids: string[]; onDone: () => void; onSelectAll: () => void }) {
  const bulk = useBulk(onDone);
  const [dialog, setDialog] = useState<null | "tags" | "location" | "time">(null);
  const run = (b: Omit<BulkBody, "photo_ids">) => bulk.mutate({ photo_ids: ids, ...b });

  return (
    <>
      <div className="flex flex-wrap items-center gap-2 border-t border-line bg-s1 px-4 py-2.5">
        <span className="mr-2 text-sm font-medium text-ink">{ids.length} selected</span>
        <Button size="sm" variant="ghost" onClick={onSelectAll}>Select all loaded</Button>
        <div className="mx-2 h-5 w-px bg-line" />
        <Button size="sm" onClick={() => setDialog("tags")}><Tag size={13} /> Tags</Button>
        <Button size="sm" onClick={() => setDialog("location")}><MapPin size={13} /> Location</Button>
        <Button size="sm" onClick={() => setDialog("time")}><Clock size={13} /> Shift time</Button>
        <Button size="sm" onClick={() => run({ edits: [{ field: "category", value: "bird" }] })}><Bird size={13} /> Mark bird</Button>
        <Button size="sm" onClick={() => run({ edits: [{ field: "category", value: "general" }] })}><ImageIcon size={13} /> Mark general</Button>
        <div className="flex items-center">
          {[1, 2, 3, 4, 5].map((r) => (
            <button key={r} title={`Rate ${r}`} onClick={() => run({ edits: [{ field: "rating", value: r }] })} className="p-1 text-muted hover:text-[#eda100]">
              <Star size={15} />
            </button>
          ))}
        </div>
        {bulk.isPending && <span className="text-xs text-muted">Saving…</span>}
        {bulk.isError && <span className="text-xs text-danger">{String(bulk.error)}</span>}
        <Button size="sm" variant="ghost" className="ml-auto" onClick={onDone}><X size={14} /> Done</Button>
      </div>

      <Modal open={dialog === "tags"} onClose={() => setDialog(null)} title={`Tags for ${ids.length} photos`}>
        <TagForm
          onSubmit={(add, remove) => {
            run({ add_tags: add, remove_tags: remove });
            setDialog(null);
          }}
        />
      </Modal>
      <Modal open={dialog === "location"} onClose={() => setDialog(null)} title={`Set location for ${ids.length} photos`} wide>
        <LocationForm
          onSubmit={(lat, lon, place) => {
            const edits: Edit[] = [{ field: "gps_lat", value: lat }, { field: "gps_lon", value: lon }];
            if (place) edits.push({ field: "place_name", value: place });
            run({ edits });
            setDialog(null);
          }}
        />
      </Modal>
      <Modal open={dialog === "time"} onClose={() => setDialog(null)} title={`Shift capture time of ${ids.length} photos`}>
        <TimeShiftForm
          onSubmit={(secs) => {
            run({ time_shift_seconds: secs });
            setDialog(null);
          }}
        />
      </Modal>
    </>
  );
}

export function Modal({ open, onClose, title, children, wide }: { open: boolean; onClose: () => void; title: string; children: ReactNode; wide?: boolean }) {
  return (
    <Dialog.Root open={open} onOpenChange={(o) => !o && onClose()}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-40 bg-black/60" />
        <Dialog.Content className={`fixed left-1/2 top-1/2 z-50 w-[calc(100vw-32px)] -translate-x-1/2 -translate-y-1/2 rounded-xl border border-line bg-s1 p-5 shadow-2xl ${wide ? "max-w-2xl" : "max-w-md"}`}>
          <Dialog.Title className="mb-4 text-base font-semibold text-ink">{title}</Dialog.Title>
          <Dialog.Description className="sr-only">{title}</Dialog.Description>
          {children}
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

function TagForm({ onSubmit }: { onSubmit: (add: string[], remove: string[]) => void }) {
  const [add, setAdd] = useState("");
  const [remove, setRemove] = useState("");
  const split = (s: string) => s.split(",").map((t) => t.trim()).filter(Boolean);
  return (
    <form className="space-y-3" onSubmit={(e) => { e.preventDefault(); onSubmit(split(add), split(remove)); }}>
      <label className="block text-sm text-ink2">Add tags (comma separated)<Input autoFocus value={add} onChange={(e) => setAdd(e.target.value)} placeholder="backyard, feeder" className="mt-1" /></label>
      <label className="block text-sm text-ink2">Remove tags<Input value={remove} onChange={(e) => setRemove(e.target.value)} className="mt-1" /></label>
      <div className="flex justify-end"><Button variant="primary" type="submit">Apply</Button></div>
    </form>
  );
}

function LocationForm({ onSubmit }: { onSubmit: (lat: number, lon: number, place?: string) => void }) {
  const [pos, setPos] = useState<{ lat: number; lon: number } | null>(null);
  const [place, setPlace] = useState("");
  return (
    <div className="space-y-3">
      <LocationPicker lat={pos?.lat} lon={pos?.lon} onChange={(lat, lon, label) => { setPos({ lat, lon }); if (label) setPlace(label); }} height={320} />
      <label className="block text-sm text-ink2">Place name (optional)<Input value={place} onChange={(e) => setPlace(e.target.value)} placeholder="e.g. Backyard feeder" className="mt-1" /></label>
      <div className="flex items-center justify-between">
        <span className="text-xs tabular-nums text-muted">{pos ? `${pos.lat.toFixed(5)}, ${pos.lon.toFixed(5)}` : "No point chosen"}</span>
        <Button variant="primary" disabled={!pos} onClick={() => pos && onSubmit(pos.lat, pos.lon, place.trim() || undefined)}>Apply</Button>
      </div>
    </div>
  );
}

function TimeShiftForm({ onSubmit }: { onSubmit: (secs: number) => void }) {
  const [sign, setSign] = useState<1 | -1>(1);
  const [d, setD] = useState(0);
  const [h, setH] = useState(0);
  const [m, setM] = useState(0);
  const secs = sign * (d * 86400 + h * 3600 + m * 60);
  const num = (v: number, set: (n: number) => void, label: string) => (
    <label className="text-sm text-ink2">{label}<Input type="number" min={0} value={v} onChange={(e) => set(Math.max(0, Number(e.target.value) || 0))} className="mt-1 w-24" /></label>
  );
  return (
    <div className="space-y-4">
      <p className="text-sm text-ink2">Use this when the camera clock was wrong or set to another time zone. Photos without a date are skipped.</p>
      <div className="flex items-end gap-3">
        <div className="flex flex-col text-sm text-ink2">Direction
          <div className="mt-1 flex">
            <Button variant={sign === 1 ? "primary" : "default"} onClick={() => setSign(1)}>Later +</Button>
            <Button variant={sign === -1 ? "primary" : "default"} onClick={() => setSign(-1)} className="ml-1">Earlier −</Button>
          </div>
        </div>
        {num(d, setD, "Days")}
        {num(h, setH, "Hours")}
        {num(m, setM, "Minutes")}
      </div>
      <div className="flex justify-end"><Button variant="primary" disabled={secs === 0} onClick={() => onSubmit(secs)}>Shift</Button></div>
    </div>
  );
}
