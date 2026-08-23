import { FormEvent, memo, useCallback, useEffect, useRef, useState } from "react";
import {
  startSearch,
  getSearchResults,
  getBackends,
  stopSearch,
  addTransfer,
  formatBytes,
  ApiError,
} from "../api/client";
import type { SearchResult } from "../api/types";

const MAX_RESULTS = 100;

type HookState = "unknown" | "ready" | "disabled" | "unconfigured";

function resultMagnet(r: SearchResult): string {
  const fromMeta = r.backend_metadata?.magnet;
  const fromAttr = r.attributes?.magnet;
  const magnet = fromMeta ?? fromAttr;
  return typeof magnet === "string" ? magnet : "";
}

function extractMagnets(text: string): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const tok of text.split(/[\s,]+/)) {
    const t = tok.trim();
    if (t.startsWith("magnet:") && !seen.has(t)) {
      seen.add(t);
      out.push(t);
    }
  }
  return out;
}

const ResultRow = memo(function ResultRow({
  r,
  downloading,
  status,
  onDownload,
}: {
  r: SearchResult;
  downloading: string | null;
  status?: string;
  onDownload: (r: SearchResult) => void;
}) {
  const busy = downloading === r.result_id;
  const magnet = resultMagnet(r);
  const seeders = typeof r.attributes?.seeders === "number" ? r.attributes.seeders : null;
  const leechers = typeof r.attributes?.leechers === "number" ? r.attributes.leechers : null;
  return (
    <tr style={{ borderTop: "1px solid var(--border)" }}>
      <td>{r.filename || "Magnet"}</td>
      <td className="muted">{formatBytes(r.size)}</td>
      <td className="muted" style={{ whiteSpace: "nowrap" }}>
        {seeders != null ? seeders : "—"}
        <span className="muted" style={{ margin: "0 4px" }}>/</span>
        {leechers != null ? leechers : "—"}
      </td>
      <td className="muted" title={magnet} style={{ maxWidth: 360, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
        {magnet || "—"}
      </td>
      <td style={{ whiteSpace: "nowrap", textAlign: "right" }}>
        <button className="btn" onClick={() => onDownload(r)} disabled={busy || !magnet}>
          {busy && <span className="spinner" aria-hidden="true" />}
          {busy ? "…" : "Download"}
        </button>
        {status && (
          <div className="muted" style={{ marginTop: 4, color: status.startsWith("Queued") ? "var(--success)" : "var(--text-muted)" }}>
            {status}
          </div>
        )}
      </td>
    </tr>
  );
});

export default function MagnetSearch() {
  const [query, setQuery] = useState("");
  const [starting, setStarting] = useState(false);
  const [loading, setLoading] = useState(false);
  const [searchId, setSearchId] = useState<string | null>(null);
  const [results, setResults] = useState<SearchResult[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [hookState, setHookState] = useState<HookState>("unknown");
  const [downloading, setDownloading] = useState<string | null>(null);
  const [downloadMsg, setDownloadMsg] = useState<Record<string, string>>({});
  const abortRef = useRef<AbortController | null>(null);

  const [ingest, setIngest] = useState("");
  const [ingesting, setIngesting] = useState(false);
  const [ingestMsg, setIngestMsg] = useState<string | null>(null);

  // Determine whether the hook is registered (command configured at startup)
  // and enabled, so the UI can show an accurate status instead of a generic
  // "disabled" message.
  useEffect(() => {
    getBackends()
      .then((list) => {
        const hook = list.find((b) => b.backend === "hook");
        if (!hook) setHookState("unconfigured");
        else setHookState(hook.search_available ? "ready" : "disabled");
      })
      .catch(() => setHookState("unknown"));
  }, []);

  const cancelFetch = useCallback(() => {
    abortRef.current?.abort();
    abortRef.current = null;
  }, []);

  async function fetchResults(id: string) {
    if (loading) return;
    setLoading(true);
    const ctrl = new AbortController();
    abortRef.current = ctrl;
    try {
      const list = await getSearchResults(id, ctrl.signal);
      setResults(list);
      if (list.length > 0) setNotice(`${list.length} magnet(s) found`);
    } catch (e) {
      if (ctrl.signal.aborted) return;
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      if (abortRef.current === ctrl) abortRef.current = null;
      setLoading(false);
    }
  }

  async function onSubmit(e: FormEvent) {
    e.preventDefault();
    const term = query.trim();
    if (!term || starting) return;
    setError(null);
    setNotice(null);
    setStarting(true);
    cancelFetch();
    if (searchId) {
      try {
        await stopSearch(searchId);
      } catch {
        /* best-effort */
      }
    }
    try {
      const { search_id } = await startSearch(term, { backend: "hook", max_results: MAX_RESULTS });
      setSearchId(search_id);
      setResults([]);
      setNotice("Search finished");
      await fetchResults(search_id);
    } catch (err) {
      // Refresh status so the message reflects the actual cause.
      if (err instanceof ApiError && err.code === "BackendUnavailable") {
        try {
          const list = await getBackends();
          const hook = list.find((b) => b.backend === "hook");
          setHookState(hook ? (hook.search_available ? "ready" : "disabled") : "unconfigured");
        } catch {
          setHookState("unknown");
        }
      } else {
        setError(err instanceof Error ? err.message : String(err));
      }
    } finally {
      setStarting(false);
    }
  }

  function onStop() {
    cancelFetch();
    if (searchId) {
      stopSearch(searchId).catch(() => {});
    }
    setNotice("Search stopped");
  }

  async function onDownload(result: SearchResult) {
    const magnet = resultMagnet(result);
    if (!magnet) return;
    setDownloading(result.result_id);
    setError(null);
    setDownloadMsg((m) => ({ ...m, [result.result_id]: "Queuing…" }));
    try {
      const resp = await addTransfer({
        backend: "torrent",
        source: magnet,
        display_name: result.filename || undefined,
      });
      setDownloadMsg((m) => ({
        ...m,
        [result.result_id]: `Queued (${resp.transfer_id.slice(0, 8)}…)`,
      }));
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setDownloadMsg((m) => ({ ...m, [result.result_id]: msg }));
      setError(msg);
    } finally {
      setDownloading(null);
    }
  }

  async function onIngest(e: FormEvent) {
    e.preventDefault();
    const magnets = extractMagnets(ingest);
    if (magnets.length === 0) {
      setError("No magnet links found. Paste one or more `magnet:?xt=urn:btih:...` links.");
      return;
    }
    setIngesting(true);
    setError(null);
    setIngestMsg(null);
    let added = 0;
    let failed = 0;
    let firstErr: string | null = null;
    for (const m of magnets) {
      try {
        await addTransfer({ backend: "torrent", source: m });
        added += 1;
      } catch (err) {
        failed += 1;
        if (!firstErr) firstErr = err instanceof Error ? err.message : String(err);
      }
    }
    setIngestMsg(
      added > 0
        ? `Added ${added} magnet(s) to downloads${failed > 0 ? `, ${failed} failed.` : "."}`
        : "None added.",
    );
    if (firstErr && failed > 0) setError(firstErr);
    if (added > 0) setIngest("");
    setIngesting(false);
  }

  return (
    <div>
      <div className="card">
        <h2>Paste magnet links</h2>
        <p className="muted">Add magnets straight to the torrent client (spaces, commas, or newlines separate multiple).</p>
        <form className="form-row" onSubmit={onIngest}>
          <input
            placeholder="magnet:?xt=urn:btih:... magnet:?xt=urn:btih:..."
            value={ingest}
            onChange={(e) => setIngest(e.target.value)}
            style={{ flex: 1 }}
          />
          <button type="submit" className="btn" disabled={ingesting || !ingest.trim()}>
            {ingesting ? "Adding…" : "Add to downloads"}
          </button>
        </form>
        {ingestMsg && <p className="muted">{ingestMsg}</p>}
      </div>

      <div className="card">
        <h2>Magnet search</h2>
        <p className="muted">
          Runs the built-in magnet search (generic engines scoped to your
          configured domains, plus optional site templates) against the query.
          Downloading a result adds it as a torrent transfer.
        </p>
        {hookState === "unconfigured" && (
          <div className="muted" style={{ color: "var(--danger)" }}>
            Magnet search is not available. The hook backend is not registered;
            restart the core.
          </div>
        )}
        {hookState === "disabled" && (
          <div className="muted" style={{ color: "var(--danger)" }}>
            Magnet search is disabled. Enable it in Settings.
          </div>
        )}
        <form className="form-row" onSubmit={onSubmit}>
          <input
            placeholder="Search magnets…"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            style={{ flex: 1 }}
          />
          <button type="submit" className="btn" disabled={starting || hookState === "disabled" || hookState === "unconfigured"}>
            {starting ? "Searching…" : "Search"}
          </button>
          {searchId && (
            <button type="button" className="btn danger" onClick={onStop}>
              Clear
            </button>
          )}
        </form>
        {error && (
          <div className="muted" style={{ color: "var(--danger)" }}>
            {error}
          </div>
        )}
        {notice && <div className="muted">{notice}</div>}
        {starting && (
          <div className="search-status">
            <span className="spinner" aria-hidden="true" />
            Running magnet search…
          </div>
        )}
      </div>

      {results.length > 0 && (
        <div className="card" style={{ padding: 0, overflow: "auto" }}>
          <table style={{ width: "100%", borderCollapse: "collapse" }}>
            <thead>
              <tr className="muted" style={{ textAlign: "left", boxShadow: "inset 0 -1px 0 var(--border)" }}>
                <th>Title</th>
                <th>Size</th>
                <th>Seeds / Leeches</th>
                <th>Magnet</th>
                <th style={{ textAlign: "right" }}>Actions</th>
              </tr>
            </thead>
            <tbody>
              {results.map((r) => (
                <ResultRow
                  key={r.result_id}
                  r={r}
                  downloading={downloading}
                  status={downloadMsg[r.result_id]}
                  onDownload={onDownload}
                />
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
