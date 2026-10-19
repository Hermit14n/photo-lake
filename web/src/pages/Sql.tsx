import { useMutation } from "@tanstack/react-query";
import { Play } from "lucide-react";
import { useState } from "react";
import { Button, Card } from "../components/ui";
import { api } from "../lib/api";

const EXAMPLES: [string, string][] = [
  ["Species by first sighting", "SELECT common_name, MIN(captured_local) AS first_seen, COUNT(*) AS photos\nFROM birds_mem b JOIN photos_mem p USING (photo_id)\nWHERE common_name IS NOT NULL\nGROUP BY common_name ORDER BY first_seen"],
  ["Uncertain identifications", "SELECT photo_id, common_name, ROUND(final_score, 2) AS score\nFROM birds_mem WHERE species_source = 'model' AND final_score < 0.3\nORDER BY score"],
  ["Lens vs. bird hit rate", "SELECT lens, COUNT(*) AS photos,\n  ROUND(AVG(CASE WHEN category = 'bird' THEN 1.0 ELSE 0 END), 2) AS bird_share\nFROM photos_mem GROUP BY lens ORDER BY photos DESC"],
  ["Edit history", "SELECT field, op, value, created_at FROM lake.photolake.annotations\nORDER BY created_at DESC LIMIT 50"],
  ["Iceberg snapshots (time travel)", "SELECT * FROM lake.photolake.\"photos$snapshots\""],
];

type Result = { columns: string[]; rows: Record<string, unknown>[]; elapsed_ms: number };

export function Sql() {
  const [q, setQ] = useState(EXAMPLES[0][1]);
  const run = useMutation({ mutationFn: (query: string) => api.post<Result>("/sql", { query }) });
  return (
    <div className="h-full overflow-auto p-3 md:p-6">
      <div className="mx-auto max-w-6xl space-y-4">
        <Card title="Query the lakehouse" action={<span className="text-xs text-muted">Read-only · DataFusion SQL · tables: photos_mem, birds_mem, lake.photolake.*</span>}>
          <div className="mb-2 flex flex-wrap gap-1.5">
            {EXAMPLES.map(([label, sql]) => <Button key={label} size="sm" variant="ghost" onClick={() => setQ(sql)}>{label}</Button>)}
          </div>
          <textarea value={q} onChange={(e) => setQ(e.target.value)} spellCheck={false} rows={7}
            onKeyDown={(e) => { if ((e.metaKey || e.ctrlKey) && e.key === "Enter") run.mutate(q); }}
            className="w-full rounded-lg border border-line bg-s2 p-3 font-mono text-[13px] text-ink outline-none focus:border-focus" />
          <div className="mt-2 flex items-center gap-3">
            <Button variant="primary" onClick={() => run.mutate(q)} disabled={run.isPending}><Play size={14} /> Run</Button>
            <span className="text-xs text-muted">Ctrl+Enter</span>
            {run.data && <span className="text-xs text-ink2">{run.data.rows.length} rows · {run.data.elapsed_ms} ms</span>}
          </div>
          {run.isError && <pre className="mt-3 whitespace-pre-wrap rounded-lg bg-s2 p-3 text-sm text-danger">{String(run.error)}</pre>}
        </Card>
        {run.data && run.data.rows.length > 0 && (
          <div className="overflow-auto rounded-xl border border-line bg-s1">
            <table className="w-full text-sm">
              <thead className="sticky top-0 bg-s2"><tr>{run.data.columns.map((c) => <th key={c} className="whitespace-nowrap px-3 py-2 text-left text-xs font-medium text-ink2">{c}</th>)}</tr></thead>
              <tbody>
                {run.data.rows.map((r, i) => (
                  <tr key={i} className="border-t border-line">
                    {run.data!.columns.map((c) => <td key={c} className="max-w-80 truncate whitespace-nowrap px-3 py-1.5 text-ink">{r[c] === null ? <span className="text-muted">null</span> : String(r[c])}</td>)}
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>
    </div>
  );
}
