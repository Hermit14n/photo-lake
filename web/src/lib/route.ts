import { useEffect, useState } from "react";
import { FILTER_KEYS, type Filters } from "./api";

/** Hash routes: #/library?category=bird&photo=<id>. Filters live in the URL so views are shareable. */
export type Page = "library" | "map" | "analytics" | "imports" | "sql" | "settings";
export type Route = { page: Page; params: URLSearchParams };

const PAGES: Page[] = ["library", "map", "analytics", "imports", "sql", "settings"];

function parse(): Route {
  const raw = window.location.hash.replace(/^#\/?/, "");
  const [path, query = ""] = raw.split("?");
  const page = (PAGES as string[]).includes(path) ? (path as Page) : "library";
  return { page, params: new URLSearchParams(query) };
}

export function useRoute(): Route {
  const [route, setRoute] = useState(parse);
  useEffect(() => {
    const on = () => setRoute(parse());
    window.addEventListener("hashchange", on);
    return () => window.removeEventListener("hashchange", on);
  }, []);
  return route;
}

export function navigate(page: Page, params: URLSearchParams | Record<string, string | undefined> = {}) {
  const p = params instanceof URLSearchParams ? params : new URLSearchParams();
  if (!(params instanceof URLSearchParams)) {
    for (const [k, v] of Object.entries(params)) if (v !== undefined && v !== "") p.set(k, v);
  }
  const q = p.toString();
  window.location.hash = `/${page}${q ? `?${q}` : ""}`;
}

export function filtersFrom(params: URLSearchParams): Filters {
  const f: Filters = {};
  for (const k of FILTER_KEYS) {
    const v = params.get(k);
    if (v) f[k] = v;
  }
  return f;
}

/** Replace filters on the current page, keeping other params (e.g. `photo`). */
export function setParams(route: Route, changes: Record<string, string | undefined>) {
  const p = new URLSearchParams(route.params);
  for (const [k, v] of Object.entries(changes)) {
    if (v === undefined || v === "") p.delete(k);
    else p.set(k, v);
  }
  navigate(route.page, p);
}
