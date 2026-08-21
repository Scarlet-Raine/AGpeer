import { FormEvent, memo, useCallback, useEffect, useRef, useState } from "react";
import {
  startSearch,
  getSearchResults,
  stopSearch,
  downloadResult,
  formatBytes,
  ApiError,
} from "../api/client";
import type { SearchResult } from "../api/types";

// Cap results requested from the backend and rendered. A search can return
// thousands of hits; showing/painting them all pegs the renderer (and, over a
// remote session, hogs the GPU). A bounded page is plenty for browsing.
const MAX_RESULTS = 200;
const POLL_MS = 2000;

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
  return (
    <tr style={{ borderTop: "1px solid var(--border)" }}>
      <td className="muted">{r.username}</td>
      <td>{r.filename}</td>
      <td className="muted">{formatBytes(r.size)}</td>
      <td>
        <span className="badge">{(r.extension ?? "").toUpperCase() || "—"}</span>
      </td>
      <td className="muted">{r.queue_length ?? "—"}</td>
      <td className="muted">{r.upload_speed ? formatBytes(r.upload_speed) + "/s" : "—"}</td>
      <td style={{ whiteSpace: "nowrap" }}>
        <button className="btn" onClick={() => onDownload(r)} disabled={busy}>
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

export default function Search() {
  const [query, setQuery] = useState("");
  const [extension, setExtension] = useState("");
  const [minSizeMb, setMinSizeMb] = useState("");
  const [searchId, setSearchId] = useState<string | null>(null);
  const [starting, setStarting] = useState(false);
  const [active, setActive] = useState(false);
  const [results, setResults] = useState<SearchResult[]>([]);
  const [total, setTotal] = useState(0);
  const [hideBusy, setHideBusy] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [downloading, setDownloading] = useState<string | null>(null);
  const [downloadMsg, setDownloadMsg] = useState<Record<string, string>>({});
  const [elapsed, setElapsed] = useState(0);
  const pollRef = useRef<number | null>(null);
  const inFlightRef = useRef(false);
  const abortRef = useRef<AbortController | null>(null);
  const lastSigRef = useRef<string | null>(null);

  const stopPolling = useCallback(() => {
    if (pollRef.current !== null) {
      window.clearInterval(pollRef.current);
      pollRef.current = null;
    }
  }, []);

  // Cancel any in-flight results fetch (e.g. a slow/large one) so it can't
  // leave inFlightRef stuck and block a newer search.
  const cancelFetch = useCallback(() => {
    if (abortRef.current) {
      abortRef.current.abort();
      abortRef.current = null;
    }
    inFlightRef.current = false;
  }, []);

  const poll = useCallback(async (id: string) => {
    if (inFlightRef.current) return;
    inFlightRef.current = true;
    const ctrl = new AbortController();
    abortRef.current = ctrl;
    try {
      const list = await getSearchResults(id, ctrl.signal);
      if (abortRef.current === ctrl) abortRef.current = null;
      // Only commit when the result set actually changed, so an idle search
      // (no new hits) doesn't re-render the table on every poll.
      const sig = `${list.length}:${list.length ? list[0].result_id : ""}`;
      if (sig !== lastSigRef.current) {
        lastSigRef.current = sig;
        setResults(list);
        setTotal(list.length);
      }
    } catch (e) {
      if (ctrl.signal.aborted) {
        // Superseded by a new search or Stop — ignore quietly.
        return;
      }
      // A timeout/transient error must not kill the search: keep polling so a
      // later attempt can succeed. Only stop for terminal backend responses
      // (e.g. the search is gone or we lost access).
      const fatal =
        e instanceof ApiError && (e.status === 401 || e.status === 403 || e.status === 404);
      setError(e instanceof Error ? e.message : String(e));
      if (fatal) {
        setActive(false);
        stopPolling();
      }
    } finally {
      if (abortRef.current === ctrl) abortRef.current = null;
      inFlightRef.current = false;
    }
  }, [stopPolling]);

  useEffect(() => {
    return () => {
      stopPolling();
      cancelFetch();
    };
  }, [stopPolling, cancelFetch]);

  // Live "searching…" clock: resets on each new/stopped search, ticks while
  // the search is active so it's obvious the search is still running.
  useEffect(() => {
    if (!active) return;
    setElapsed(0);
    const t = window.setInterval(() => setElapsed((e) => e + 1), 1000);
    return () => window.clearInterval(t);
  }, [active]);

  const seconds = (n: number) => (n < 60 ? `${n}s` : `${Math.floor(n / 60)}m ${n % 60}s`);

  async function onSubmit(e: FormEvent) {
    e.preventDefault();
    const term = query.trim();
    if (!term || starting) return;
    setError(null);
    setNotice(null);
    setStarting(true);
    lastSigRef.current = null;
    cancelFetch();

    // If a search is already running, replace it: stop polling and tell the
    // backend to stop the old search before starting the new one.
    stopPolling();
    if (active && searchId) {
      try {
        await stopSearch(searchId);
      } catch {
        /* best-effort; the new search replaces it anyway */
      }
    }
    setActive(false);

    try {
      const opts = {
        extension: extension.trim() || undefined,
        min_size: minSizeMb ? Math.round(parseFloat(minSizeMb) * 1024 * 1024) : undefined,
        max_results: MAX_RESULTS,
      };
      const { search_id } = await startSearch(term, opts);
      setSearchId(search_id);
      setResults([]);
      setTotal(0);
      setActive(true);
      setNotice(`Search "${term}" started`);
      // Clear "Starting…" as soon as the search is accepted, then poll. Do the
      // first fetch async so a slow results fetch can never block the state
      // transitions or the poll loop it started earlier.
      setStarting(false);
      pollRef.current = window.setInterval(() => void poll(search_id), POLL_MS);
      void poll(search_id);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setStarting(false);
    }
  }

  async function onStop() {
    stopPolling();
    cancelFetch();
    setActive(false);
    if (searchId) {
      try {
        await stopSearch(searchId);
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err));
      }
    }
    setNotice("Search stopped");
  }

  async function onDownload(result: SearchResult) {
    if (!searchId) return;
    setDownloading(result.result_id);
    setError(null);
    setDownloadMsg((m) => ({ ...m, [result.result_id]: "Queuing…" }));
    try {
      const resp = await downloadResult(searchId, result.result_id);
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

  const visible = hideBusy
    ? results.filter((r) => r.free_upload_slots !== false)
    : results;
  const filteredOut = results.length - visible.length;

  return (
    <div>
      <div className="card">
        <h2>Soulseek search</h2>
        <form className="form-row" onSubmit={onSubmit}>
          <input
            placeholder="Search the Soulseek network…"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            style={{ flex: 1 }}
          />
          <input
            placeholder="ext (e.g. flac)"
            value={extension}
            onChange={(e) => setExtension(e.target.value)}
            style={{ width: 110 }}
          />
          <input
            placeholder="min MB"
            value={minSizeMb}
            onChange={(e) => setMinSizeMb(e.target.value)}
            style={{ width: 90 }}
          />
          <label
            className="muted"
            style={{ display: "flex", alignItems: "center", gap: 6, whiteSpace: "nowrap" }}
            title="Hide results from peers that report no free upload slots (busy / likely-not-shared)."
          >
            <input
              type="checkbox"
              checked={hideBusy}
              onChange={(e) => setHideBusy(e.target.checked)}
            />
            Hide busy peers
          </label>
          <button type="submit" className="btn" disabled={starting}>
            {starting ? "Starting…" : active ? "Search again" : "Search"}
          </button>
          {active && (
            <button type="button" className="btn danger" onClick={onStop}>
              Stop
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
            Starting search…
          </div>
        )}
        {active && !starting && (
          <div className={`search-status ${total > 0 ? "hit" : "waiting"}`}>
            <span className={`pulse ${total > 0 ? "" : "waiting"}`} aria-hidden="true" />
            {total > 0 ? (
              <>
                <strong>Searching…</strong>{" "}
                <span className="search-status-meta">
                  {hideBusy ? `${visible.length} of ${total} hit(s) so far` : `${total} hit(s) so far`}
                  {total > MAX_RESULTS ? ` (first ${MAX_RESULTS} shown)` : ""}
                  {hideBusy && filteredOut > 0 ? ` · hid ${filteredOut} busy` : ""} · {seconds(elapsed)}
                </span>
              </>
            ) : (
              <>
                <strong>No results yet</strong>{" "}
                <span className="search-status-meta">still searching the network · {seconds(elapsed)}</span>
              </>
            )}
          </div>
        )}
      </div>

      {visible.length > 0 && (
        <div className="card">
          <table style={{ width: "100%", borderCollapse: "collapse" }}>
            <thead>
              <tr className="muted" style={{ textAlign: "left" }}>
                <th>User</th>
                <th>Filename</th>
                <th>Size</th>
                <th>Format</th>
                <th>Queue</th>
                <th>Speed</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {visible.map((r) => (
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
