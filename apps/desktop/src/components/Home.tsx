import { FormEvent, useEffect, useState } from "react";
import { getStatus, listTransfers, addTransfer, formatBytes } from "../api/client";
import type { BackendStatus, Transfer } from "../api/types";

export default function Home() {
  const [backends, setBackends] = useState<BackendStatus[]>([]);
  const [transfers, setTransfers] = useState<Transfer[]>([]);
  const [magnet, setMagnet] = useState("");
  const [adding, setAdding] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [ok, setOk] = useState<string | null>(null);

  useEffect(() => {
    getStatus()
      .then((s) => setBackends(s.backends))
      .catch((e) => setError(String(e)));
    listTransfers()
      .then((t) => setTransfers(t))
      .catch(() => {});
  }, []);

  async function onSubmit(e: FormEvent) {
    e.preventDefault();
    if (!magnet.trim()) return;
    setAdding(true);
    setError(null);
    setOk(null);
    try {
      const r = await addTransfer({ backend: "torrent", source: magnet.trim() });
      setOk(`Added ${r.transfer_id}`);
      setMagnet("");
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setAdding(false);
    }
  }

  const totalBytes = transfers.reduce(
    (sum, t) => sum + (t.bytes_total ?? 0),
    0,
  );

  return (
    <div>
      <div className="card">
        <h2>Add transfer</h2>
        <form className="form-row" onSubmit={onSubmit}>
          <input
            placeholder="magnet:?xt=urn:btih:..."
            value={magnet}
            onChange={(e) => setMagnet(e.target.value)}
            style={{ flex: 1 }}
          />
          <button type="submit" className="btn" disabled={adding}>
            {adding ? "Adding…" : "Add magnet"}
          </button>
        </form>
        {ok && <p className="muted">{ok}</p>}
        {error && (
          <p className="muted" style={{ color: "#e5484d" }}>
            {error}
          </p>
        )}
      </div>

      <div className="card">
        <h2>Backends</h2>
        {backends.length === 0 ? (
          <p className="muted">No backend status available.</p>
        ) : (
          backends.map((b) => (
            <p key={b.backend}>
              {b.backend} — transfer:{" "}
              {b.transfer_available ? "yes" : "no"} / search:{" "}
              {b.search_available ? "yes" : "no"}
            </p>
          ))
        )}
      </div>

      <div className="card">
        <h2>Summary</h2>
        <p>
          {transfers.length} transfers · {formatBytes(totalBytes)}
        </p>
      </div>
    </div>
  );
}
