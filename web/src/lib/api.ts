export type Filters = {
  category?: string;
  species?: string;
  camera?: string;
  lens?: string;
  place?: string;
  tag?: string;
  from?: string;
  to?: string;
  year?: string;
  month?: string;
  hour?: string;
  undated?: string;
  has_location?: string;
  edited?: string;
  rating_min?: string;
  batch?: string;
  q?: string;
  sort?: string;
};

export const FILTER_KEYS: (keyof Filters)[] = [
  "category", "species", "camera", "lens", "place", "tag", "from", "to", "year", "month",
  "hour", "undated", "has_location", "edited", "rating_min", "batch", "q", "sort",
];

export type PhotoSummary = {
  photo_id: string;
  captured_at: string | null;
  category: "bird" | "general";
  species: string | null;
  bird_boxes: number;
  width: number | null;
  height: number | null;
  rating: number | null;
  place_name: string | null;
  original_filename: string;
  title: string | null;
  has_location: boolean;
  edited: boolean;
};

export type Bird = {
  photo_id: string;
  box_index: number;
  bbox_x: number;
  bbox_y: number;
  bbox_w: number;
  bbox_h: number;
  det_score: number | null;
  species_code: string | null;
  common_name: string | null;
  scientific_name: string | null;
  final_score: number | null;
  species_source: "user" | "model" | null;
  confirmed: boolean;
  user_added: boolean;
};

export type Candidate = {
  box_index: number;
  rank: number;
  species_code: string;
  common_name: string;
  scientific_name: string;
  cls_score: number;
  final_score: number;
  model_version: string;
};

export type HistoryRow = { target: string; field: string; op: string; value: string | null; created_at: string };

export type PhotoDetail = {
  photo: Record<string, any>;
  original: Record<string, any> | null;
  birds: Bird[];
  candidates: Candidate[];
  custom: Record<string, string>;
  history: HistoryRow[];
};

export type Facet = { value: string | number; n: number; label?: string };
export type Facets = Record<"cameras" | "lenses" | "places" | "categories" | "years" | "species" | "tags", Facet[]>;

export type Edit = { target?: string; field: string; op?: "set" | "clear" | "revert"; value?: unknown };

export type SpeciesHit = { code: string; common: string; sci: string; family: string };

export function toQuery(f: Filters, extra: Record<string, string | number | undefined> = {}): string {
  const p = new URLSearchParams();
  for (const k of FILTER_KEYS) {
    const v = f[k];
    if (v !== undefined && v !== "") p.set(k, v);
  }
  for (const [k, v] of Object.entries(extra)) if (v !== undefined) p.set(k, String(v));
  const s = p.toString();
  return s ? `?${s}` : "";
}

export type Me = { auth_required: boolean; has_viewer: boolean; role: "admin" | "viewer" | null };

import { createContext, useContext } from "react";

/** The signed-in role; viewers get a read-only UI (the server enforces it too). */
export const RoleContext = createContext<"admin" | "viewer">("admin");
export const useReadOnly = () => useContext(RoleContext) === "viewer";

/** Fired when the session is missing or expired, so the app can show the sign-in screen. */
export const UNAUTHORIZED_EVENT = "photolake:unauthorized";

async function handle<T>(r: Response): Promise<T> {
  if (r.status === 401) window.dispatchEvent(new Event(UNAUTHORIZED_EVENT));
  if (!r.ok) {
    let msg = `${r.status} ${r.statusText}`;
    try {
      const body = await r.json();
      if (body?.error) msg = body.error;
    } catch {
      /* not JSON */
    }
    throw new Error(msg);
  }
  if (r.status === 204) return undefined as T;
  return r.json() as Promise<T>;
}

export const api = {
  get: <T,>(path: string) => fetch(`/api${path}`).then((r) => handle<T>(r)),
  post: <T,>(path: string, body: unknown) =>
    fetch(`/api${path}`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    }).then((r) => handle<T>(r)),
  put: <T,>(path: string, body: unknown) =>
    fetch(`/api${path}`, {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    }).then((r) => handle<T>(r)),
};

export const img = (id: string, kind: "thumb" | "preview" | "full" | "raw" | "original") => `/api/img/${id}/${kind}`;

/** Format a stored UTC timestamp in the viewer's local time. */
export function fmtDate(ts: string | null | undefined, opts: Intl.DateTimeFormatOptions = { dateStyle: "medium", timeStyle: "short" }) {
  if (!ts) return "No date";
  const d = new Date(ts.replace(" ", "T"));
  return isNaN(d.getTime()) ? ts : d.toLocaleString(undefined, opts);
}

export function localDayKey(ts: string | null): string {
  if (!ts) return "undated";
  const d = new Date(ts.replace(" ", "T"));
  if (isNaN(d.getTime())) return "undated";
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}
