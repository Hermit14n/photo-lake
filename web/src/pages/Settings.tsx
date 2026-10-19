import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import clsx from "clsx";
import { Cloud, Folder, HardDrive, KeyRound, Play, Plus, Save, Trash2 } from "lucide-react";
import { Fragment, useEffect, useState, type ReactNode } from "react";
import { Button, Card, Input, Select, Spinner } from "../components/ui";
import { api, fmtDate, type Me } from "../lib/api";

type Target = { name: string; enabled: boolean; kind: "local"; fs_uuid?: string | null; path: string } | { name: string; enabled: boolean; kind: "rclone"; remote: string };
type RunResult = { ok: boolean; error: string | null; finished_at: string | null; files_copied: number; bytes_copied: number; files_total: number; location: string | null };
type BackupStatus = { running: boolean; rclone: string | null; targets: { target: Target; available: boolean; location: string | null; reason: string | null; last: RunResult | null }[] };
type SettingsT = {
  config_path: string | null;
  import: { auto_import: boolean; camera_timezone: string | null };
  ml: { enabled: boolean; detection_threshold: number; has_ebird_key: boolean; home_region: string | null; tiling: boolean; device: string };
  backup: { after_import: boolean; daily_at: string | null; targets: Target[] };
};
type Drive = { device: string; mount_point: string; uuid: string | null; label: string | null; total_bytes: number | null; free_bytes: number | null; is_camera_card: boolean };

const gb = (b: number | null | undefined) => (b == null ? "?" : `${(b / 1e9).toFixed(b > 1e11 ? 0 : 1)} GB`);

export function Settings() {
  const qc = useQueryClient();
  const settings = useQuery({ queryKey: ["settings"], queryFn: () => api.get<SettingsT>("/settings") });
  const backup = useQuery({ queryKey: ["backup"], queryFn: () => api.get<BackupStatus>("/backup"), refetchInterval: (q) => (q.state.data?.running ? 2000 : 15000) });
  const status = useQuery({ queryKey: ["status"], queryFn: () => api.get<Record<string, unknown>>("/status") });
  const save = useMutation({
    mutationFn: (patch: Record<string, unknown>) => api.put<{ restart_required: boolean }>("/settings", patch),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["settings"] });
      qc.invalidateQueries({ queryKey: ["backup"] });
    },
  });
  const run = useMutation({ mutationFn: (target?: string) => api.post("/backup/run", { target }), onSuccess: () => setTimeout(() => qc.invalidateQueries({ queryKey: ["backup"] }), 500) });

  if (!settings.data) return <div className="flex justify-center py-24"><Spinner /></div>;
  const s = settings.data;

  return (
    <div className="h-full overflow-auto p-3 md:p-6">
      <div className="mx-auto max-w-4xl space-y-4">
        {save.data?.restart_required && <div className="rounded-lg border border-accent/50 bg-accent/10 px-4 py-2.5 text-sm text-ink">Saved. Restart Photolake for model/time-zone changes to take effect.</div>}
        {save.isError && <div className="rounded-lg border border-danger/50 px-4 py-2.5 text-sm text-danger">{String(save.error)}</div>}

        <Card title="Backups" action={<Button size="sm" variant="primary" disabled={backup.data?.running || !s.backup.targets.length} onClick={() => run.mutate(undefined)}><Play size={13} /> Back up now</Button>}>
          <p className="mb-3 text-sm text-ink2">
            Everything is backed up: originals, previews, the Iceberg tables and a consistent catalog snapshot. Backups only ever add files, so deleting something in the library never deletes it from a backup.
          </p>
          <div className="space-y-2">
            {backup.data?.targets.map((t) => (
              <div key={t.target.name} className="flex items-center gap-3 rounded-lg border border-line bg-s2 p-3">
                <div className="text-ink2">{t.target.kind === "rclone" ? <Cloud size={18} /> : t.target.fs_uuid ? <HardDrive size={18} /> : <Folder size={18} />}</div>
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2 text-sm font-medium text-ink">
                    {t.target.name}
                    {!t.target.enabled && <span className="rounded bg-s3 px-1.5 text-[11px] font-normal text-muted">disabled</span>}
                    <span className={clsx("rounded px-1.5 text-[11px] font-normal", t.available ? "bg-[#0ca30c]/15 text-[#0ca30c]" : "bg-s3 text-muted")}>{t.available ? "● ready" : "○ unavailable"}</span>
                  </div>
                  <div className="truncate text-xs text-muted">{t.location ?? t.reason}</div>
                  {t.last && (
                    <div className={clsx("text-xs", t.last.ok ? "text-ink2" : "text-danger")}>
                      Last backup {fmtDate(t.last.finished_at)} · {t.last.ok ? `${t.last.files_copied} new files (${gb(t.last.bytes_copied)}), ${t.last.files_total} total` : t.last.error}
                    </div>
                  )}
                </div>
                <Button size="sm" disabled={!t.available || backup.data?.running} onClick={() => run.mutate(t.target.name)}><Play size={13} /> Run</Button>
                <Button size="sm" variant="ghost" title="Remove target" onClick={() => confirm(`Remove backup target "${t.target.name}"? Files already backed up are not deleted.`) && save.mutate({ targets: s.backup.targets.filter((x) => x.name !== t.target.name) })}><Trash2 size={13} /></Button>
              </div>
            ))}
            {backup.data?.running && <div className="flex items-center gap-2 text-sm text-ink2"><Spinner /> Backup running…</div>}
          </div>
          <AddTarget existing={s.backup.targets} rclone={backup.data?.rclone ?? null} onAdd={(t) => save.mutate({ targets: [...s.backup.targets, t] })} />
          <div className="mt-4 flex flex-wrap items-center gap-4 border-t border-line pt-4 text-sm">
            <Toggle label="Back up after every import" checked={s.backup.after_import} onChange={(v) => save.mutate({ after_import: v })} />
            <DailyTime value={s.backup.daily_at} onSave={(v) => save.mutate({ daily_at: v })} />
          </div>
        </Card>

        <SignInCard />

        <Card title="Import">
          <div className="space-y-3 text-sm">
            <Toggle label="Import automatically when a camera card is inserted" checked={s.import.auto_import} onChange={(v) => save.mutate({ auto_import: v })} />
            <TextSetting label="Camera time zone" hint="Used for cameras that don't record a UTC offset. Leave empty to use this computer's zone." value={s.import.camera_timezone ?? ""} placeholder="e.g. America/Denver" onSave={(v) => save.mutate({ camera_timezone: v })} />
          </div>
        </Card>

        <Card title="Bird identification">
          <div className="space-y-3 text-sm">
            <Toggle label="Detect and identify birds" checked={s.ml.enabled} onChange={(v) => save.mutate({ ml_enabled: v })} />
            <TextSetting label="eBird API key" password hint={<>Optional, free at <a className="text-focus underline" href="https://ebird.org/api/keygen" target="_blank" rel="noreferrer">ebird.org/api/keygen</a>. Ranks species by what's actually seen near the photo, like Merlin does. {s.ml.has_ebird_key && <b className="text-[#0ca30c]">Key saved.</b>}</>} value="" placeholder={s.ml.has_ebird_key ? "•••••••• (saved; type to replace)" : "paste key"} onSave={(v) => save.mutate({ ebird_api_key: v })} />
            <TextSetting label="Home region" hint="eBird region used for photos without GPS (your camera has none), e.g. US-CO, US-CO-031, GB-ENG." value={s.ml.home_region ?? ""} placeholder="US-CO" onSave={(v) => save.mutate({ home_region: v })} />
            <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
              <span className="w-full text-ink2 sm:w-44">Detection threshold</span>
              <input type="range" min={0.1} max={0.9} step={0.05} defaultValue={s.ml.detection_threshold} onPointerUp={(e) => save.mutate({ detection_threshold: Number((e.target as HTMLInputElement).value) })} className="w-48 accent-[var(--accent)]" />
              <span className="tabular-nums text-ink">{s.ml.detection_threshold.toFixed(2)}</span>
              <span className="text-xs text-muted">lower finds more birds, with more false alarms</span>
            </div>
            <Toggle label="Search tiles for small/distant birds when none are found (slower)" checked={s.ml.tiling} onChange={(v) => save.mutate({ tiling: v })} />
            <Rerun device={(status.data?.ml_device as string | null | undefined) ?? null} />
          </div>
        </Card>

        <Card title="System">
          {!status.data ? <Spinner /> : (
            <dl className="grid grid-cols-1 gap-y-1.5 text-sm sm:grid-cols-[180px_1fr]">
              {[...Object.entries(status.data), ["config_file", s.config_path ?? "—"], ["rclone", backup.data?.rclone ?? "not installed (needed only for cloud targets)"]].map(([k, v]) => (
                <Fragment key={String(k)}><dt className="text-ink2">{String(k).replace(/_/g, " ")}</dt><dd className="truncate text-ink">{String(v)}</dd></Fragment>
              ))}
            </dl>
          )}
        </Card>
      </div>
    </div>
  );
}

function Toggle({ label, checked, onChange }: { label: string; checked: boolean; onChange: (v: boolean) => void }) {
  return (
    <label className="flex cursor-pointer items-center gap-2.5 text-sm text-ink">
      <button role="switch" aria-checked={checked} onClick={() => onChange(!checked)} className={clsx("relative h-5 w-9 rounded-full transition-colors", checked ? "bg-accent" : "bg-s3")}>
        <span className={clsx("absolute top-0.5 h-4 w-4 rounded-full bg-white transition-all", checked ? "left-[18px]" : "left-0.5")} />
      </button>
      {label}
    </label>
  );
}

function TextSetting({ label, hint, value, placeholder, password, onSave }: { label: string; hint?: ReactNode; value: string; placeholder?: string; password?: boolean; onSave: (v: string) => void }) {
  const [v, setV] = useState(value);
  useEffect(() => setV(value), [value]);
  return (
    <div className="flex flex-col gap-1 sm:flex-row sm:items-start sm:gap-3">
      <span className="shrink-0 text-ink2 sm:w-44 sm:pt-2">{label}</span>
      <div className="flex-1">
        <div className="flex gap-2">
          <Input type={password ? "password" : "text"} value={v} placeholder={placeholder} onChange={(e) => setV(e.target.value)} className="max-w-sm" />
          <Button disabled={v === value} onClick={() => onSave(v)}><Save size={14} /> Save</Button>
        </div>
        {hint && <p className="mt-1 text-xs text-muted">{hint}</p>}
      </div>
    </div>
  );
}

function DailyTime({ value, onSave }: { value: string | null; onSave: (v: string) => void }) {
  const [v, setV] = useState(value ?? "");
  return (
    <span className="flex items-center gap-2 whitespace-nowrap text-ink">
      Daily backup at
      <Input type="time" value={v} onChange={(e) => setV(e.target.value)} onBlur={() => v !== (value ?? "") && onSave(v)} className="h-8 w-32" />
      <span className="text-xs text-muted">(empty = off)</span>
    </span>
  );
}

function AddTarget({ existing, rclone, onAdd }: { existing: Target[]; rclone: string | null; onAdd: (t: Target) => void }) {
  const [open, setOpen] = useState(false);
  const [kind, setKind] = useState<"drive" | "folder" | "cloud">("drive");
  const [name, setName] = useState("");
  const [uuid, setUuid] = useState<string>();
  const [path, setPath] = useState("photolake-backup");
  const [remote, setRemote] = useState("");
  const drives = useQuery({ queryKey: ["drives"], queryFn: () => api.get<Drive[]>("/drives"), enabled: open, refetchInterval: open ? 3000 : false });
  const candidates = (drives.data ?? []).filter((d) => d.uuid && !d.is_camera_card);
  const nameTaken = existing.some((t) => t.name === name.trim());
  const valid = name.trim() && !nameTaken && (kind === "drive" ? !!uuid && path.trim() : kind === "folder" ? path.startsWith("/") : remote.includes(":"));

  if (!open) return <Button className="mt-3" onClick={() => setOpen(true)}><Plus size={14} /> Add backup target</Button>;
  return (
    <div className="mt-3 space-y-3 rounded-lg border border-line p-4">
      <div className="flex gap-2">
        {([["drive", <HardDrive size={14} />, "External drive"], ["folder", <Folder size={14} />, "Folder"], ["cloud", <Cloud size={14} />, "Cloud (rclone)"]] as const).map(([k, icon, label]) => (
          <Button key={k} variant={kind === k ? "primary" : "default"} onClick={() => { setKind(k); if (k === "folder") setPath(""); else if (k === "drive") setPath("photolake-backup"); }}>{icon} {label}</Button>
        ))}
      </div>
      <label className="block text-sm text-ink2">Name<Input value={name} onChange={(e) => setName(e.target.value)} placeholder={kind === "cloud" ? "backblaze" : "external-hdd"} className="mt-1 max-w-xs" />{nameTaken && <span className="text-xs text-danger">name already used</span>}</label>
      {kind === "drive" && (
        <>
          <div className="text-sm text-ink2">
            Drive — plug it in and pick it here. It's remembered by its filesystem ID, so backups run whenever it's connected, wherever it mounts.
            <div className="mt-1">
              {drives.isLoading ? <Spinner /> : candidates.length === 0 ? <span className="text-xs text-muted">No external drives detected yet.</span> : (
                <Select value={uuid} onChange={setUuid} allowAny={false} placeholder="Choose a drive" options={candidates.map((d) => ({ value: d.uuid!, label: `${d.label ?? d.mount_point} — ${gb(d.free_bytes)} free of ${gb(d.total_bytes)}` }))} className="min-w-80" />
              )}
            </div>
          </div>
          <label className="block text-sm text-ink2">Folder on the drive<Input value={path} onChange={(e) => setPath(e.target.value)} className="mt-1 max-w-xs" /></label>
        </>
      )}
      {kind === "folder" && <label className="block text-sm text-ink2">Absolute folder path (NAS mount, second disk…)<Input value={path} onChange={(e) => setPath(e.target.value)} placeholder="/mnt/nas/photolake" className="mt-1 max-w-md" /></label>}
      {kind === "cloud" && (
        <div className="text-sm text-ink2">
          rclone remote and path — works with Backblaze B2, S3, Cloudflare R2, Google Drive, Dropbox, OneDrive, SFTP and more. Set the remote up once with <code className="rounded bg-s3 px-1">rclone config</code>.
          {!rclone && <div className="mt-1 text-xs text-danger">rclone isn't installed: <code>sudo apt install rclone</code></div>}
          <Input value={remote} onChange={(e) => setRemote(e.target.value)} placeholder="b2:my-bucket/photolake" className="mt-1 max-w-md" />
        </div>
      )}
      <div className="flex justify-end gap-2">
        <Button variant="ghost" onClick={() => setOpen(false)}>Cancel</Button>
        <Button variant="primary" disabled={!valid} onClick={() => {
          onAdd(kind === "cloud" ? { name: name.trim(), enabled: true, kind: "rclone", remote: remote.trim() } : { name: name.trim(), enabled: true, kind: "local", fs_uuid: kind === "drive" ? uuid : null, path: path.trim() });
          setOpen(false);
        }}>Add target</Button>
      </div>
    </div>
  );
}

/** Admin and viewer passwords. Required before other devices (Wi-Fi, Tailscale) can connect. */
function SignInCard() {
  const qc = useQueryClient();
  const me = useQuery({ queryKey: ["me"], queryFn: () => api.get<Me>("/auth/me"), staleTime: Infinity });
  const [which, setWhich] = useState<"admin" | "viewer" | null>(null);
  const [pw, setPw] = useState("");
  const [pw2, setPw2] = useState("");
  const set = useMutation({
    mutationFn: (b: { password?: string; viewer: boolean; clear?: boolean }) => api.post("/auth/password", b),
    onSuccess: () => {
      setWhich(null);
      setPw("");
      setPw2("");
      qc.invalidateQueries({ queryKey: ["me"] });
    },
  });
  const m = me.data;
  if (!m) return null;
  const mismatch = pw2.length > 0 && pw !== pw2;
  return (
    <Card title="Sign-in & remote access">
      <div className="space-y-3 text-sm">
        <p className="text-ink2">
          {m.auth_required
            ? "A password protects Photolake. Phones and other computers sign in with it."
            : "No password yet: only this computer can open Photolake. Set one before turning on Wi-Fi or Tailscale access (photolake-ctl remote on)."}
        </p>
        <div className="flex flex-wrap items-center gap-2">
          <Button size="sm" variant={m.auth_required ? undefined : "primary"} onClick={() => setWhich("admin")}><KeyRound size={13} /> {m.auth_required ? "Change password" : "Set password"}</Button>
          {m.auth_required && (
            <Button size="sm" onClick={() => setWhich("viewer")}><KeyRound size={13} /> {m.has_viewer ? "Change viewer password" : "Add view-only password"}</Button>
          )}
          {m.has_viewer && (
            <Button size="sm" variant="ghost" onClick={() => confirm("Remove the view-only password? Anyone signed in with it is signed out.") && set.mutate({ viewer: true, clear: true })}>Remove viewer password</Button>
          )}
        </div>
        {which && (
          <form
            className="flex flex-wrap items-end gap-2 rounded-lg border border-line bg-s2 p-3"
            onSubmit={(e) => {
              e.preventDefault();
              if (!mismatch) set.mutate({ password: pw, viewer: which === "viewer" });
            }}
          >
            <label className="flex flex-col gap-1 text-xs text-ink2">
              {which === "viewer" ? "View-only password" : "New password"}
              <Input type="password" autoComplete="new-password" value={pw} onChange={(e) => setPw(e.target.value)} />
            </label>
            <label className="flex flex-col gap-1 text-xs text-ink2">
              Repeat
              <Input type="password" autoComplete="new-password" value={pw2} onChange={(e) => setPw2(e.target.value)} />
            </label>
            <Button size="sm" variant="primary" disabled={pw.length < 8 || pw !== pw2 || set.isPending}><Save size={13} /> Save</Button>
            <Button size="sm" variant="ghost" onClick={() => setWhich(null)}>Cancel</Button>
            <div className="w-full text-xs text-muted">{mismatch ? <span className="text-danger">Passwords don't match.</span> : "At least 8 characters. Other devices will need to sign in again."}</div>
          </form>
        )}
        {set.isError && <div className="text-danger">{String(set.error)}</div>}
      </div>
    </Card>
  );
}

type ReprocessEvent = { kind: "reprocess"; running: boolean; done: number; total: number; birds: number; error: string | null };

/** Re-run bird ML over the library (after a model update or a settings change). */
function Rerun({ device }: { device: string | null }) {
  const [p, setP] = useState<ReprocessEvent | null>(null);
  useEffect(() => {
    const es = new EventSource("/api/events");
    es.onmessage = (m) => {
      const ev = JSON.parse(m.data);
      if (ev.kind === "reprocess") setP(ev);
    };
    return () => es.close();
  }, []);
  const start = useMutation({ mutationFn: (all: boolean) => api.post("/reprocess", { all }) });
  const running = p?.running || start.isPending;
  return (
    <div className="space-y-2 border-t border-line pt-3">
      <div className="text-ink2">
        Runs on: <b className="text-ink">{device === "cuda" ? "GPU (CUDA)" : device === "cpu" ? "CPU" : "models not loaded"}</b>
        {device === "cpu" && <span className="text-muted"> · an NVIDIA GPU makes this ~10x faster</span>}
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <Button size="sm" disabled={!device || running} onClick={() => start.mutate(false)}><Play size={13} /> Analyze photos not yet checked</Button>
        <Button size="sm" variant="ghost" disabled={!device || running} onClick={() => confirm("Re-run bird detection on every photo? Your corrections are kept.") && start.mutate(true)}>Re-run on all photos</Button>
        {p && (
          <span className={clsx("text-xs", p.error ? "text-danger" : "text-muted")}>
            {p.error ? p.error : p.running ? `${p.done} / ${p.total} photos · ${p.birds} with birds` : `Done: ${p.done} photos, ${p.birds} with birds`}
          </span>
        )}
        {start.isError && <span className="text-xs text-danger">{String(start.error)}</span>}
      </div>
    </div>
  );
}
