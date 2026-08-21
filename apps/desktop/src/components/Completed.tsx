import { useCallback, useEffect, useState } from "react";
import { deleteTransfer, formatBytes, listTransfers, stateLabel } from "../api/client";
import { subscribeEvents } from "../api/events";
import type { Transfer } from "../api/types";

const DONE: string[] = ["completed", "ready", "failed", "cancelled"];

function DoneCard({ t, onRemove }: { t: Transfer; onRemove: (id: string, deleteData: boolean) => void }) {
  return (
    <div className="card" key={t.id}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: 12 }}>
        <strong>{t.display_name}</strong>
        <span className="badge" style={{ color: "var(--state-" + t.state + ")" }}>
          {stateLabel(t.state)}
        </span>
      </div>
      <div className="muted">{t.destination}</div>
      {t.error && <div className="muted" style={{ color: "var(--danger)" }}>{t.error}</div>}
      <div style={{ display: "flex", gap: 8, alignItems: "center", marginTop: 8 }}>
        <span className="badge">{t.backend}</span>
        <span className="badge">post: {t.postprocess_state}</span>
        {t.bytes_total ? <span className="muted">{formatBytes(t.bytes_total)}</span> : null}
        <div style={{ marginLeft: "auto" }}>
          <button className="btn danger" onClick={() => onRemove(t.id, true)}>Remove + files</button>
          <button className="btn" style={{ marginLeft: 8 }} onClick={() => onRemove(t.id, false)}>Remove</button>
        </div>
      </div>
    </div>
  );
}

export default function Completed() {
  const [transfers, setTransfers] = useState<Transfer[]>([]);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const all = await listTransfers();
      const done = all
        .filter((t) => DONE.includes(t.state))
        .sort((a, b) => (b.completed_at ?? b.created_at).localeCompare(a.completed_at ?? a.created_at));
      setTransfers(done);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, []);

  useEffect(() => {
    void refresh();
    const off = subscribeEvents((ev) => {
      if (ev.kind.startsWith("transfer.") || ev.kind.startsWith("postprocess.")) void refresh();
    });
    return off;
  }, [refresh]);

  async function remove(id: string, deleteData: boolean) {
    if (!confirm(deleteData ? "Delete the completed transfer record AND its files?" : "Delete the completed transfer record?")) return;
    try {
      await deleteTransfer(id, deleteData);
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  const torrents = transfers.filter((t) => t.backend === "torrent");
  const soulseek = transfers.filter((t) => t.backend === "soulseek");

  const section = (title: string, list: Transfer[]) =>
    list.length > 0 ? (
      <section style={{ marginBottom: 16 }}>
        <h3>{title}</h3>
        {list.map((t) => <DoneCard key={t.id} t={t} onRemove={remove} />)}
      </section>
    ) : null;

  return (
    <div>
      <h2>Completed</h2>
      {error && <div className="muted" style={{ color: "var(--danger)" }}>{error}</div>}
      {torrents.length === 0 && soulseek.length === 0 && (
        <div className="card muted">No completed downloads yet.</div>
      )}
      {section("Torrents", torrents)}
      {section("Soulseek", soulseek)}
    </div>
  );
}
