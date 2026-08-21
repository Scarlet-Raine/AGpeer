import type { AppEvent } from "./types";
import { API_BASE, getApiToken } from "./client";

// Events are { kind, payload, ts }. Kinds include transfer.added|started|progress|paused|resumed|completed|failed|cancelled|removed|orphaned,
// backend.ready|degraded, search.*, postprocess.*.
export type ConnectionStatus = "connecting" | "connected" | "disconnected";

export function connectEvents(
  onEvent: (e: AppEvent) => void,
  onStatus: (s: ConnectionStatus) => void,
): () => void {
  let stop = false;
  let attempt = 0;

  async function run(): Promise<void> {
    onStatus("connecting");
    const token = await getApiToken();
    if (!token) {
      console.warn("[agpeer] no API token available; scheduling reconnect");
      onStatus("disconnected");
      scheduleReconnect();
      return;
    }
    let res: Response | undefined;
    try {
      res = await fetch(`${API_BASE}/api/v1/events`, {
        headers: { Authorization: `Bearer ${token}` },
      });
      if (!res.ok || !res.body) throw new Error(`SSE ${res.status}`);
    } catch (err) {
      console.warn(`[agpeer] SSE connection failed (${res?.status ?? "no response"}): ${err}`);
      onStatus("disconnected");
      scheduleReconnect();
      return;
    }
    console.info(`[agpeer] connected to event stream at ${API_BASE}`);
    onStatus("connected");
    attempt = 0;
    try {
      const reader = res.body.getReader();
      const decoder = new TextDecoder();
      let buf = "";
      for (;;) {
        const { done, value } = await reader.read();
        if (done) break;
        buf += decoder.decode(value, { stream: true });
        const blocks = buf.split("\n\n");
        buf = blocks.pop() ?? "";
        for (const block of blocks) {
          for (const line of block.split("\n")) {
            if (!line.startsWith("data:")) continue;
            try {
              onEvent(JSON.parse(line.slice(5).trim()) as AppEvent);
            } catch {
              // skip invalid payloads
            }
          }
        }
      }
    } catch (err) {
      console.warn(`[agpeer] event stream errored; reconnecting: ${err}`);
    }
    console.info("[agpeer] event stream closed; reconnecting");
    onStatus("disconnected");
    scheduleReconnect();
  }

  function scheduleReconnect(): void {
    if (stop) return;
    const delay = Math.min(30000, 1000 * 2 ** attempt);
    attempt += 1;
    console.info(`[agpeer] scheduling reconnect in ${delay}ms (attempt ${attempt})`);
    setTimeout(() => {
      if (!stop) void run();
    }, delay);
  }

  void run();

  return () => {
    stop = true;
  };
}

type EventHandler = (e: AppEvent) => void;
type StatusHandler = (s: ConnectionStatus) => void;

// A single shared event stream. Multiple components subscribe here instead of
// each opening their own SSE connection (which wasted connections and caused
// duplicate refreshes and renders, adding renderer/GPU load).
let hubStarted = false;
let hubStatus: ConnectionStatus = "connecting";
const eventHandlers = new Set<EventHandler>();
const statusHandlers = new Set<StatusHandler>();

export function subscribeEvents(onEvent: EventHandler, onStatus?: StatusHandler): () => void {
  if (!hubStarted) startHub();
  eventHandlers.add(onEvent);
  if (onStatus) {
    statusHandlers.add(onStatus);
    onStatus(hubStatus);
  }
  return () => {
    eventHandlers.delete(onEvent);
    if (onStatus) statusHandlers.delete(onStatus);
  };
}

function startHub(): void {
  hubStarted = true;
  connectEvents(
    (e) => {
      for (const h of eventHandlers) h(e);
    },
    (s) => {
      hubStatus = s;
      for (const h of statusHandlers) h(s);
    },
  );
}
