import { useQuery, useQueryClient } from "@tanstack/react-query";
import clsx from "clsx";
import * as Dialog from "@radix-ui/react-dialog";
import { BarChart3, Bird, Database, FolderInput, Images, LogOut, Map as MapIcon, Menu, Moon, Settings as SettingsIcon, Sun } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import { PhotoDetail } from "./components/PhotoDetail";
import { api, RoleContext, UNAUTHORIZED_EVENT, type Me } from "./lib/api";
import { filtersFrom, navigate, setParams, useRoute, type Page } from "./lib/route";
import { Analytics } from "./pages/Analytics";
import { Imports, LiveProgress } from "./pages/Imports";
import { Library } from "./pages/Library";
import { Login } from "./pages/Login";
import { MapPage } from "./pages/MapPage";
import { Settings } from "./pages/Settings";
import { Sql } from "./pages/Sql";

export type LiveImport = { running: boolean; source: string; done: number; total: number; imported: number; birds: number; last?: string };

/** Server-sent events: live import progress, and cache invalidation when data changes. */
function useEvents() {
  const qc = useQueryClient();
  const [live, setLive] = useState<LiveImport | null>(null);
  const [backup, setBackup] = useState<Record<string, unknown> | null>(null);
  useEffect(() => {
    const es = new EventSource("/api/events");
    let debounce: ReturnType<typeof setTimeout> | undefined;
    const refresh = () => {
      clearTimeout(debounce);
      debounce = setTimeout(() => qc.invalidateQueries(), 400);
    };
    es.onmessage = (m) => {
      const ev = JSON.parse(m.data);
      if (ev.kind === "data_changed") refresh();
      if (ev.kind === "backup") {
        setBackup(ev.status);
        qc.invalidateQueries({ queryKey: ["backup"] });
      }
      if (ev.kind !== "import") return;
      const e = ev.event;
      if (e.type === "started") setLive({ running: true, source: e.source, done: 0, total: e.total, imported: 0, birds: 0 });
      else if (e.type === "progress")
        setLive((l) => ({
          running: true,
          source: l?.source ?? "",
          done: e.done,
          total: e.total,
          imported: (l?.imported ?? 0) + (e.outcome === "bird" || e.outcome === "general" ? 1 : 0),
          birds: (l?.birds ?? 0) + (e.outcome === "bird" ? 1 : 0),
          last: `${e.file}: ${e.outcome}`,
        }));
      else if (e.type === "finished") {
        setLive((l) => (l ? { ...l, running: false, imported: e.batch.photos_imported, birds: e.batch.bird_count } : l));
        refresh();
      } else if (e.type === "failed") setLive((l) => (l ? { ...l, running: false, last: `Failed: ${e.error}` } : l));
    };
    return () => es.close();
  }, [qc]);
  return { live, backup };
}

function useThemeToggle() {
  const [theme, setTheme] = useState<"dark" | "light" | null>(() => {
    try {
      return (localStorage.getItem("photolake.theme") as "dark" | "light" | null) ?? null;
    } catch {
      return null;
    }
  });
  useEffect(() => {
    if (theme) document.documentElement.dataset.theme = theme;
    else delete document.documentElement.dataset.theme;
    try {
      theme ? localStorage.setItem("photolake.theme", theme) : localStorage.removeItem("photolake.theme");
    } catch {
      /* storage unavailable */
    }
  }, [theme]);
  const isDark = theme ? theme === "dark" : window.matchMedia("(prefers-color-scheme: dark)").matches;
  return { isDark, toggle: () => setTheme(isDark ? "light" : "dark") };
}

const NAV: { page: Page; label: string; icon: ReactNode; admin?: boolean }[] = [
  { page: "library", label: "Library", icon: <Images size={18} /> },
  { page: "map", label: "Map", icon: <MapIcon size={18} /> },
  { page: "analytics", label: "Analytics", icon: <BarChart3 size={18} /> },
  { page: "imports", label: "Imports", icon: <FolderInput size={18} />, admin: true },
  { page: "sql", label: "SQL", icon: <Database size={18} />, admin: true },
  { page: "settings", label: "Settings & backup", icon: <SettingsIcon size={18} />, admin: true },
];

/** Sign-in gate: shows the login screen until the server accepts the session. */
export function App() {
  const qc = useQueryClient();
  const me = useQuery({ queryKey: ["me"], queryFn: () => api.get<Me>("/auth/me"), staleTime: Infinity });
  useEffect(() => {
    const onUnauthorized = () => qc.invalidateQueries({ queryKey: ["me"] });
    window.addEventListener(UNAUTHORIZED_EVENT, onUnauthorized);
    return () => window.removeEventListener(UNAUTHORIZED_EVENT, onUnauthorized);
  }, [qc]);
  if (!me.data) return null;
  if (me.data.auth_required && !me.data.role) return <Login onSignedIn={() => qc.resetQueries()} />;
  const role = me.data.role ?? "admin";
  return (
    <RoleContext.Provider value={role}>
      <Shell role={role} authRequired={me.data.auth_required} />
    </RoleContext.Provider>
  );
}

function Shell({ role, authRequired }: { role: "admin" | "viewer"; authRequired: boolean }) {
  const qc = useQueryClient();
  const route = useRoute();
  const filters = filtersFrom(route.params);
  const photo = route.params.get("photo");
  const { live } = useEvents();
  const { isDark, toggle } = useThemeToggle();
  // Carry filters between the views that share them.
  const shared = new Set<Page>(["library", "map", "analytics"]);
  const go = (page: Page) => {
    if (shared.has(page) && shared.has(route.page)) {
      const p = new URLSearchParams(route.params);
      p.delete("photo");
      navigate(page, p);
    } else navigate(page);
  };

  const [menuOpen, setMenuOpen] = useState(false);
  const nav = (
    <NavContent
      role={role}
      authRequired={authRequired}
      current={route.page}
      live={live}
      isDark={isDark}
      onToggleTheme={toggle}
      onGo={(page) => {
        setMenuOpen(false);
        go(page);
      }}
      onSignOut={async () => {
        await api.post("/auth/logout", {});
        qc.resetQueries();
      }}
    />
  );

  return (
    <div className="flex h-dvh flex-col md:flex-row">
      <nav className="hidden w-56 shrink-0 flex-col border-r border-line bg-s1 md:flex">{nav}</nav>
      {/* Phones: a slim top bar; the same navigation opens as a drawer. */}
      <Dialog.Root open={menuOpen} onOpenChange={setMenuOpen}>
        <header className="flex shrink-0 items-center gap-2 border-b border-line bg-s1 px-2 pb-1.5 pt-[max(0.375rem,env(safe-area-inset-top))] md:hidden">
          <Dialog.Trigger className="rounded-lg p-2 text-ink2 hover:bg-s2 hover:text-ink" aria-label="Menu">
            <Menu size={20} />
          </Dialog.Trigger>
          <div className="flex h-7 w-7 items-center justify-center rounded-lg bg-accent text-white"><Bird size={16} /></div>
          <div className="text-[15px] font-semibold text-ink">{NAV.find((n) => n.page === route.page)?.label ?? "Photolake"}</div>
          {live?.running && route.page !== "imports" && (
            <button className="ml-auto truncate pr-2 text-xs text-ink2" onClick={() => navigate("imports")}>
              Importing {live.done}/{live.total}
            </button>
          )}
        </header>
        <Dialog.Portal>
          <Dialog.Overlay className="fixed inset-0 z-40 bg-black/60 md:hidden" />
          <Dialog.Content className="fixed inset-y-0 left-0 z-50 flex w-64 max-w-[85vw] flex-col border-r border-line bg-s1 pt-[env(safe-area-inset-top)] pb-[env(safe-area-inset-bottom)] shadow-2xl md:hidden">
            <Dialog.Title className="sr-only">Navigation</Dialog.Title>
            <Dialog.Description className="sr-only">Pages and account</Dialog.Description>
            {nav}
          </Dialog.Content>
        </Dialog.Portal>
      </Dialog.Root>
      <main className="min-h-0 min-w-0 flex-1">
        {route.page === "library" && <Library route={route} filters={filters} />}
        {route.page === "map" && <MapPage route={route} filters={filters} />}
        {route.page === "analytics" && <Analytics route={route} filters={filters} />}
        {role === "admin" && route.page === "imports" && <Imports live={live} />}
        {role === "admin" && route.page === "sql" && <Sql />}
        {role === "admin" && route.page === "settings" && <Settings />}
      </main>
      {photo && (
        <PhotoDetail
          id={photo}
          filters={filters}
          onClose={() => setParams(route, { photo: undefined })}
          onNavigate={(id) => setParams(route, { photo: id })}
        />
      )}
    </div>
  );
}

function NavContent({
  role,
  authRequired,
  current,
  live,
  isDark,
  onToggleTheme,
  onGo,
  onSignOut,
}: {
  role: "admin" | "viewer";
  authRequired: boolean;
  current: Page;
  live: LiveImport | null;
  isDark: boolean;
  onToggleTheme: () => void;
  onGo: (page: Page) => void;
  onSignOut: () => void;
}) {
  return (
    <>
      <div className="flex items-center gap-2 px-5 py-5">
        <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-accent text-white"><Bird size={18} /></div>
        <div>
          <div className="text-[15px] font-semibold leading-tight text-ink">Photolake</div>
          <div className="text-[11px] text-muted">Iceberg photo lakehouse</div>
        </div>
      </div>
      <div className="flex-1 space-y-0.5 overflow-y-auto px-2">
        {NAV.filter((n) => role === "admin" || !n.admin).map((n) => (
          <button key={n.page} onClick={() => onGo(n.page)} className={clsx("flex w-full items-center gap-3 rounded-lg px-3 py-2.5 text-sm md:py-2", current === n.page ? "bg-s3 font-medium text-ink" : "text-ink2 hover:bg-s2 hover:text-ink")}>
            {n.icon} {n.label}
          </button>
        ))}
      </div>
      {live?.running && current !== "imports" && (
        <button className="mx-3 mb-3 rounded-lg border border-line bg-s2 p-2.5 text-left" onClick={() => onGo("imports")}>
          <LiveProgress live={live} compact />
        </button>
      )}
      <div className="border-t border-line p-3">
        <button onClick={onToggleTheme} className="flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-sm text-ink2 hover:bg-s2 hover:text-ink">
          {isDark ? <Sun size={16} /> : <Moon size={16} />} {isDark ? "Light mode" : "Dark mode"}
        </button>
        {authRequired && (
          <button onClick={onSignOut} className="flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-sm text-ink2 hover:bg-s2 hover:text-ink">
            <LogOut size={16} /> Sign out{role === "viewer" ? " (viewer)" : ""}
          </button>
        )}
      </div>
    </>
  );
}
