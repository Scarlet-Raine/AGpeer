import { useCallback, useEffect, useState } from "react";
import {
  listTransfers,
  transferAction,
  deleteTransfer,
  formatBytes,
  formatEta,
  stateLabel,
  stateClass,
} from "../api/client";
import { subscribeEvents } from "../api/events";
import type { Transfer } from "../api/types";

const QUEUE_STATES = new Set([
  "queued",
  "resolving",
  "downloading",
  "paused",
  "verifying",
  "postprocessing",
]);

function QueueCard({
  t,
  onAction,
  onRemove,
}: {
  t: Transfer;
  onAction: (id: string, a: "pause" | "resume" | "cancel") => void;
  onRemove: (id: string) => void;
}) {
  const paused = t.state === "paused";
  const active = !paused;
  return (
    <div className="card">
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: 8 }}>
        <strong style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", flex: 1 }}>
          {t.display_name}
        </strong>
        <span className="badge">{t.backend}</span>
        <span className="badge" style={{ color: "var(--" + stateClass(t.state) + ")" }}>
          {stateLabel(t.state)}
        </span>
      </div>
      <div className="muted" style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }} title={t.source}>
        {t.source}
      </div>
      <div className="progress" style={{ marginTop: 8 }}>
        <div className="progress-fill" style={{ width: `${Math.round(t.progress * 100)}%` }} />
      </div>
      <div className="muted">
        {Math.round(t.progress * 100)}% · {formatBytes(t.bytes_completed)}
        {t.bytes_total ? ` / ${formatBytes(t.bytes_total)}` : ""}
        {active ? ` · ↓ ${formatBytes(t.download_rate || 0)} · ETA ${formatEta(t.eta)}` : ""}
      </div>
      {t.error && <div className="muted" style={{ color: "var(--danger)" }}>{t.error}</div>}
      <div style={{ display: "flex", gap: 8, marginTop: 8 }}>
        {t.state === "downloading" && (
          <button className="btn" onClick={() => onAction(t.id, "pause")}>Pause</button>
        )}
        {t.state === "paused" && (
          <button className="btn" onClick={() => onAction(t.id, "resume")}>Resume</button>
        )}
        {(t.state === "queued" || t.state === "resolving") && (
          <button className="btn" onClick={() => onAction(t.id, "cancel")}>Cancel</button>
        )}
        {(t.state === "downloading" || t.state === "paused") && (
          <button className="btn danger" onClick={() => onAction(t.id, "cancel")}>Cancel</button>
        )}
        <button className="btn" onClick={() => onRemove(t.id)}>Remove</button>
      </div>
    </div>
  );
}

export default function Queue() {
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
    if (!confirm("Delete the transfer record AND its downloaded files?")) return;
    try {
      await deleteTransfer(id, true);
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  const queue = transfers.filter((t) => QUEUE_STATES.has(t.state));
  const active = queue.filter((t) => t.state !== "paused");
  const paused = queue.filter((t) => t.state === "paused");
  const rate = active.reduce((s, t) => s + (t.download_rate || 0), 0);
  const totalBytes = queue.reduce((s, t) => s + (t.bytes_total || 0), 0);

  return (
    <div>
      <h2>Queue</h2>
      {error && <div className="muted" style={{ color: "var(--danger)" }}>{error}</div>}
      <div className="card">
        <h3>Summary</h3>
        <p className="muted">
          {queue.length} in queue · {active.length} active · {paused.length} paused ·{" "}
          {formatBytes(rate)}/s aggregate · {formatBytes(totalBytes)} total
        </p>
      </div>
      {queue.length === 0 && (
        <div className="card muted">The queue is empty. Add a magnet on Home, or start a Soulseek download.</div>
      )}
      <div className="grid">
        {queue.map((t) => (
          <QueueCard key={t.id} t={t} onAction={act} onRemove={remove} />
        ))}
      </div>
    </div>
  );
}
