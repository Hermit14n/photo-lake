import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { CheckCircle2, FolderInput, HardDriveDownload } from "lucide-react";
import { useState } from "react";
import { Button, Card, Input, Spinner } from "../components/ui";
import { api, fmtDate } from "../lib/api";
import { navigate } from "../lib/route";
import type { LiveImport } from "../App";

type Batch = { batch_id: string; started_at: string; finished_at: string; source_path: string; files_seen: number; photos_imported: number; skipped_duplicates: number; errors: number; bird_count: number };

export function Imports({ live }: { live: LiveImport | null }) {
  const qc = useQueryClient();
  const batches = useQuery({ queryKey: ["batches"], queryFn: () => api.get<Batch[]>("/imports") });
  const [path, setPath] = useState("");
  const start = useMutation({ mutationFn: (p: string) => api.post("/imports", { path: p }), onSuccess: () => qc.invalidateQueries({ queryKey: ["batches"] }) });

  return (
    <div className="h-full overflow-auto p-3 md:p-6">
      <div className="mx-auto max-w-4xl space-y-4">
        <Card title="Import">
          <p className="mb-3 text-sm text-ink2">
            Plug in your camera's card and it imports automatically. Each photo is checked for birds and saved to the lakehouse; duplicates are skipped. You can also import any folder:
          </p>
          <form className="flex gap-2" onSubmit={(e) => { e.preventDefault(); path.trim() && start.mutate(path.trim()); }}>
            <Input value={path} onChange={(e) => setPath(e.target.value)} placeholder="/media/you/CARD or ~/Pictures/trip" />
            <Button variant="primary" type="submit" disabled={!path.trim() || start.isPending || !!live?.running}><FolderInput size={15} /> Import folder</Button>
          </form>
          {start.isError && <p className="mt-2 text-sm text-danger">{String(start.error)}</p>}
          {live && <LiveProgress live={live} />}
        </Card>

        <GpxCard />

        <Card title="History">
          {batches.isLoading ? <Spinner /> : !batches.data?.length ? <p className="text-sm text-ink2">No imports yet.</p> : (
            <div className="overflow-x-auto">
            <table className="w-full whitespace-nowrap text-sm">
              <thead><tr className="text-left text-xs text-muted"><th className="py-2 font-medium">When</th><th className="font-medium">Source</th><th className="text-right font-medium">Imported</th><th className="text-right font-medium">Birds</th><th className="text-right font-medium">Skipped</th><th className="text-right font-medium">Errors</th></tr></thead>
              <tbody>
                {batches.data.map((b) => (
                  <tr key={b.batch_id} className="cursor-pointer border-t border-line hover:bg-s2" onClick={() => navigate("library", { batch: b.batch_id, sort: "imported_desc" })} title="Show this import's photos">
                    <td className="py-2 text-ink">{fmtDate(b.started_at)}</td>
                    <td className="max-w-64 truncate text-ink2">{b.source_path}</td>
                    <td className="text-right tabular-nums text-ink">{b.photos_imported}</td>
                    <td className="text-right tabular-nums text-ink2">{b.bird_count}</td>
                    <td className="text-right tabular-nums text-ink2">{b.skipped_duplicates}</td>
                    <td className={`text-right tabular-nums ${b.errors ? "text-danger" : "text-ink2"}`}>{b.errors}</td>
                  </tr>
                ))}
              </tbody>
            </table>
            </div>
          )}
        </Card>
      </div>
    </div>
  );
}

function GpxCard() {
  const qc = useQueryClient();
  const [overwrite, setOverwrite] = useState(false);
  const [gap, setGap] = useState(10);
  const up = useMutation({
    mutationFn: async (f: File) => {
      const r = await fetch(`/api/import/gpx?overwrite=${overwrite}&max_gap_minutes=${gap}`, { method: "POST", body: await f.text() });
      const j = await r.json();
      if (!r.ok) throw new Error(j.error ?? r.statusText);
      return j as { track_points: number; photos_located: number; track_start: string; track_end: string };
    },
    onSuccess: () => qc.invalidateQueries(),
  });
  return (
    <Card title="Add locations from a GPX track">
      <p className="mb-3 text-sm text-ink2">Your camera doesn't record GPS. Record a track on your phone (Strava, Gaia GPS, OsmAnd, Google Fit export…) while you shoot, then drop the .gpx here — photos are matched by time.</p>
      <div className="flex flex-wrap items-center gap-4 text-sm text-ink">
        <input type="file" accept=".gpx,application/gpx+xml" onChange={(e) => e.target.files?.[0] && up.mutate(e.target.files[0])} className="text-sm text-ink2 file:mr-3 file:rounded-lg file:border file:border-line file:bg-s2 file:px-3 file:py-1.5 file:text-ink" />
        <label className="flex items-center gap-2">Max gap <Input type="number" min={1} value={gap} onChange={(e) => setGap(Number(e.target.value) || 10)} className="h-8 w-20" /> min</label>
        <label className="flex items-center gap-2"><input type="checkbox" checked={overwrite} onChange={(e) => setOverwrite(e.target.checked)} /> Replace existing locations</label>
      </div>
      {up.isPending && <div className="mt-2"><Spinner /></div>}
      {up.isError && <p className="mt-2 text-sm text-danger">{String(up.error)}</p>}
      {up.data && <p className="mt-2 text-sm text-ink">Located <b>{up.data.photos_located}</b> photos from {up.data.track_points.toLocaleString()} track points ({fmtDate(up.data.track_start)} – {fmtDate(up.data.track_end)}).</p>}
    </Card>
  );
}

export function LiveProgress({ live, compact }: { live: LiveImport; compact?: boolean }) {
  const pct = live.total ? Math.round((live.done / live.total) * 100) : 0;
  return (
    <div className={compact ? "" : "mt-4 rounded-lg border border-line bg-s2 p-3"}>
      <div className="mb-1.5 flex items-center justify-between text-sm">
        <span className="flex items-center gap-2 text-ink">
          {live.running ? <HardDriveDownload size={15} className="text-accent" /> : <CheckCircle2 size={15} className="text-[#0ca30c]" />}
          {live.running ? `Importing ${live.source}` : `Imported ${live.imported} photos (${live.birds} with birds)`}
        </span>
        <span className="tabular-nums text-ink2">{live.done}/{live.total}</span>
      </div>
      <div className="h-1.5 overflow-hidden rounded-full bg-s3"><div className="h-full rounded-full bg-accent transition-[width]" style={{ width: `${pct}%` }} /></div>
      {!compact && live.last && <div className="mt-1.5 truncate text-xs text-muted">{live.last}</div>}
    </div>
  );
}
