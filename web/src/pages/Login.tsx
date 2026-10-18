import { Lock } from "lucide-react";
import { useState } from "react";
import { api } from "../lib/api";

export function Login({ onSignedIn }: { onSignedIn: () => void }) {
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await api.post("/auth/login", { password });
      onSignedIn();
    } catch (err) {
      setError((err as Error).message);
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="flex h-full items-center justify-center bg-bg px-4">
      <form onSubmit={submit} className="w-full max-w-sm rounded-xl border border-line bg-s1 p-6 shadow-lg">
        <div className="mb-5 flex items-center gap-3">
          <div className="flex h-10 w-10 items-center justify-center rounded-full bg-s3 text-ink">
            <Lock size={18} />
          </div>
          <div>
            <h1 className="text-lg font-semibold text-ink">Photolake</h1>
            <p className="text-sm text-ink2">Sign in to view your library</p>
          </div>
        </div>
        <label className="mb-1 block text-sm text-ink2" htmlFor="pw">
          Password
        </label>
        <input
          id="pw"
          type="password"
          autoFocus
          autoComplete="current-password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          className="w-full rounded-lg border border-line bg-s2 px-3 py-2 text-ink outline-none focus:border-accent"
        />
        {error && <p className="mt-2 text-sm text-danger">{error}</p>}
        <button type="submit" disabled={busy || !password} className="mt-4 w-full rounded-lg bg-accent px-3 py-2 font-medium text-[var(--accent-ink)] disabled:opacity-50">
          {busy ? "Signing in…" : "Sign in"}
        </button>
      </form>
    </div>
  );
}
