import type { Backend, StatusResponse, Transfer, BackendStatus, AddTransferRequest, AddTransferResponse, TransferFile, SearchResult } from "./types";

export const API_BASE = (import.meta.env.VITE_API_URL as string | undefined) ?? "http://127.0.0.1:41000";

const REQUEST_TIMEOUT_MS = 60_000;

let cachedToken: string | null | undefined;

async function fetchDevToken(): Promise<string | null> {
  try {
    const ctrl = new AbortController();
    const to = setTimeout(() => ctrl.abort(), 2000);
    const res = await fetch("/__agpeer_token", { signal: ctrl.signal, cache: "no-store" });
    clearTimeout(to);
    if (res.ok) {
      const t = (await res.text()).trim();
      if (t) return t;
    }
  } catch {
    // Not served by a Vite dev server (e.g. the built Tauri app) — fall through.
  }
  return null;
}

export async function getApiToken(): Promise<string | null> {
  if (cachedToken !== undefined) return cachedToken;
  // (1) Live token from the Vite dev server (reads the same file the core uses).
  const live = await fetchDevToken();
  if (live) {
    cachedToken = live;
    return cachedToken;
  }
  // (2) Injected by the dev launcher for plain-browser dev (Vite env var).
  const devToken = (import.meta.env.VITE_AGPEER_TOKEN as string | undefined)?.trim();
  if (devToken) {
    cachedToken = devToken;
    return cachedToken;
  }
  // (3) Injected into the window by the Tauri shell.
  const injected = (window as unknown as { __AGPEER_TOKEN__?: string }).__AGPEER_TOKEN__;
  if (injected) {
    cachedToken = injected;
    return cachedToken;
  }
  try {
    const { invoke } = await import("@tauri-apps/api/core");
    const t = await invoke<string>("get_api_token");
    if (t) {
      cachedToken = t;
      return t;
    }
  } catch {
    // dynamic import so plain-browser dev doesn't break
  }
  cachedToken = null;
  return null;
}

export class ApiError extends Error {
  constructor(public code: string, message: string, public status: number) {
    super(message);
    this.name = "ApiError";
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const token = await getApiToken();
  const headers = new Headers(init?.headers);
  if (token) headers.set("Authorization", `Bearer ${token}`);
  if (init?.body) headers.set("Content-Type", "application/json");
  // Bound every request so a slow/hung backend (e.g. a slow soulseek search)
  // can never wedge the UI on a loader forever, and honor a caller-supplied
  // abort signal (so a new search/stop can cancel an in-flight results fetch).
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), REQUEST_TIMEOUT_MS);
  let onExternalAbort: (() => void) | undefined;
  if (init?.signal) {
    if (init.signal.aborted) {
      controller.abort();
    } else {
      onExternalAbort = () => controller.abort();
      init.signal.addEventListener("abort", onExternalAbort, { once: true });
    }
  }
  try {
    // Trailing `signal` overrides any signal carried in `init`.
    const res = await fetch(API_BASE + path, { ...init, headers, signal: controller.signal });
    if (!res.ok) {
      let code: string | undefined;
      let message: string | undefined;
      try {
        const body = await res.json();
        code = body?.code;
        message = body?.message;
      } catch {
        // best-effort body parse
      }
      throw new ApiError(code ?? `Http${res.status}`, message || res.statusText, res.status);
    }
    return res.json() as Promise<T>;
  } catch (e) {
    if (e instanceof Error && e.name === "AbortError") {
      if (init?.signal?.aborted) {
        // Cancelled by the caller (e.g. a new search replaced this one) — not
        // a timeout. Re-throw so callers can tell the difference.
        throw e;
      }
      throw new ApiError(
        "timeout",
        `The request timed out after ${Math.round(REQUEST_TIMEOUT_MS / 1000)}s. The backend may be busy or unreachable.`,
        0,
      );
    }
    throw e;
  } finally {
    clearTimeout(timer);
    if (onExternalAbort && init?.signal) {
      init.signal.removeEventListener("abort", onExternalAbort);
    }
  }
}

export const getStatus = () => request<StatusResponse>("/api/v1/status");

export const getBackends = () => request<BackendStatus[]>("/api/v1/backends");

export const listTransfers = () => request<Transfer[]>("/api/v1/transfers");

export const addTransfer = (req: AddTransferRequest) => request<AddTransferResponse>("/api/v1/transfers", { method: "POST", body: JSON.stringify(req) });

export const transferAction = (id: string, action: "pause" | "resume" | "cancel") => request<Transfer>(`/api/v1/transfers/${id}/${action}`, { method: "POST", body: JSON.stringify(action === "cancel" ? { delete_data: false } : {}) });

export const deleteTransfer = (id: string, deleteData = false) =>
  request<{ message: string }>(`/api/v1/transfers/${id}`, {
    method: "DELETE",
    body: JSON.stringify({ delete_data: deleteData }),
  });

export const getTransferFiles = (id: string) => request<TransferFile[]>(`/api/v1/transfers/${id}/files`);

export interface SearchOptions {
  backend?: Backend;
  extension?: string;
  min_size?: number;
  max_results?: number;
}

export const startSearch = (query: string, opts: SearchOptions = {}) =>
  request<{ search_id: string }>("/api/v1/searches", {
    method: "POST",
    body: JSON.stringify({ backend: opts.backend ?? "soulseek", query, extension: opts.extension ?? null, min_size: opts.min_size ?? null, max_results: opts.max_results ?? null }),
  });

export const getSearchResults = (searchId: string, signal?: AbortSignal) =>
  request<SearchResult[]>(`/api/v1/searches/${searchId}/results`, { signal });

export const getSettings = () => request<Record<string, unknown>>("/api/v1/settings");
export const putSettings = (map: Record<string, unknown>) =>
  request<Record<string, unknown>>("/api/v1/settings", {
    method: "PUT",
    body: JSON.stringify(map),
  });
export const deleteSetting = (key: string) =>
  request<{ message: string }>(`/api/v1/settings/${encodeURIComponent(key)}`, {
    method: "DELETE",
  });

export const getLibrary = () => request<{ path: string; absolute_path: string; size: number | null; is_dir: boolean }[]>("/api/v1/library");

/** Open a folder in the OS file manager (Tauri). Falls back to copying the path. */
export async function openPath(path: string): Promise<boolean> {
  try {
    const { invoke } = await import("@tauri-apps/api/core");
    await invoke("open_path", { path });
    return true;
  } catch {
    try {
      await navigator.clipboard.writeText(path);
      return false;
    } catch {
      return false;
    }
  }
}

export const stopSearch = (searchId: string) =>
  request<{ message: string }>(`/api/v1/searches/${searchId}/stop`, { method: "POST" });

export const downloadResult = (searchId: string, resultId: string) =>
  request<AddTransferResponse>(`/api/v1/searches/${searchId}/results/${resultId}/download`, {
    method: "POST",
    body: JSON.stringify({}),
  });

export function formatBytes(bytes: number | null | undefined): string {
  if (bytes === null || bytes === undefined || bytes <= 0) return "—";
  const units = ["B", "KB", "MB", "GB"];
  let value = bytes;
  let i = 0;
  while (value >= 1024 && i < units.length - 1) {
    value /= 1024;
    i += 1;
  }
  return `${value.toFixed(1)} ${units[i]}`;
}

export function formatEta(seconds: number | null | undefined): string {
  if (seconds === null || seconds === undefined) return "—";
  const s = Math.floor(seconds);
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m ${s % 60}s`;
  return `${Math.floor(s / 3600)}h ${Math.floor((s % 3600) / 60)}m`;
}

export const stateLabel = (s: string): string => s.charAt(0).toUpperCase() + s.slice(1);

export const stateClass = (s: string): string => `state-${s}`;
