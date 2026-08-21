import { useEffect, useState, useCallback } from "react";
import { listTransfers, transferAction, deleteTransfer, formatBytes, formatEta, stateLabel, stateClass } from "../api/client";
import { subscribeEvents } from "../api/events";
import type { Transfer } from "../api/types";

const ACTIVE_STATES = new Set(["queued", "resolving", "downloading", "paused", "verifying", "postprocessing"]);

function Row({ t, onAction, onRemove }: { t: Transfer; onAction: (id: string, a: "pause" | "resume" | "cancel") => void; onRemove: (id: string) => void }) {
  const active = ACTIVE_STATES.has(t.state);
  const downloading = t.state === "downloading";
  return (
    <tr style={{ borderTop: "1px solid var(--border)" }}>
      <td>
        <strong>{t.display_name}</strong>
        {t.error && <div className="muted" style={{ color: "var(--danger)" }}>{t.error}</div>}
      </td>
      <td className="muted" title={t.source} style={{ maxWidth: 220, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
        {t.source}
      </td>
      <td style={{ whiteSpace: "nowrap" }}>
        <span className="badge" style={{ color: "var(--" + stateClass(t.state) + ")" }}>{stateLabel(t.state)}</span>
        <span className="badge">{t.postprocess_state}</span>
      </td>
      <td>
        <div className="progress" style={{ minWidth: 140 }}>
          <div className="progress-fill" style={{ width: `${Math.round(t.progress * 100)}%` }} />
        </div>
        <div className="muted">{Math.round(t.progress * 100)}%{downloading && ` · ↓ ${formatBytes(t.download_rate)} · ETA ${formatEta(t.eta)}`}</div>
      </td>
      <td className="muted">{formatBytes(t.bytes_completed)}{t.bytes_total ? ` / ${formatBytes(t.bytes_total)}` : ""}</td>
      <td style={{ whiteSpace: "nowrap", textAlign: "right" }}>
        {t.state === "downloading" && <button className="btn" onClick={() => onAction(t.id, "pause")}>Pause</button>}
        {t.state === "paused" && <button className="btn" onClick={() => onAction(t.id, "resume")}>Resume</button>}
        {active && <button className="btn danger" onClick={() => onAction(t.id, "cancel")}>Cancel</button>}
        <button className="btn" onClick={() => onRemove(t.id)}>Remove</button>
      </td>
    </tr>
  );
}

function Section({ title, list, onAction, onRemove }: {
  title: string;
  list: Transfer[];
  onAction: (id: string, a: "pause" | "resume" | "cancel") => void;
  onRemove: (id: string) => void;
}) {
  if (list.length === 0) return null;
  return (
    <section style={{ marginBottom: 16 }}>
      <h3>{title}</h3>
      <div className="card" style={{ padding: 0, overflow: "auto" }}>
        <table style={{ width: "100%", borderCollapse: "collapse" }}>
          <thead>
            <tr className="muted" style={{ textAlign: "left", boxShadow: "inset 0 -1px 0 var(--border)" }}>
              <th>Name</th>
              <th>Source</th>
              <th>State</th>
              <th>Progress</th>
              <th>Size</th>
              <th style={{ textAlign: "right" }}>Actions</th>
            </tr>
          </thead>
          <tbody>
            {list.map((t) => <Row key={t.id} t={t} onAction={onAction} onRemove={onRemove} />)}
          </tbody>
        </table>
      </div>
    </section>
  );
}

export default function Downloads() {
  const [transfers, setTransfers] = useState<Transfer[]>([]);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setTransfers(await listTransfers());
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, []);

  useEffect(() => {
    void refresh();
    const off = subscribeEvents((ev) => {
      if (ev.kind.startsWith("transfer.")) void refresh();
    });
    return off;
  }, [refresh]);

  async function act(id: string, action: "pause" | "resume" | "cancel") {
    try {
      await transferAction(id, action);
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  async function remove(id: string) {
    if (!confirm("Delete the transfer record and its downloaded files?")) return;
    try {
      await deleteTransfer(id, true);
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  const magnets = transfers.filter((t) => t.backend === "torrent");
  const soulseek = transfers.filter((t) => t.backend === "soulseek");

  return (
    <div>
      <h2>Downloads</h2>
      {error && <div className="muted" style={{ color: "var(--danger)" }}>{error}</div>}
      {transfers.length === 0 && (
        <div className="card muted">No transfers yet. Add a magnet from Home or the magnet search, or search on Soulseek.</div>
      )}
      <Section title="Magnets" list={magnets} onAction={act} onRemove={remove} />
      <Section title="Soulseek" list={soulseek} onAction={act} onRemove={remove} />
    </div>
  );
}
