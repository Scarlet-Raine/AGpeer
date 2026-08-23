export type Backend = "torrent" | "soulseek" | "hook";

export type ExtractStrategy = "table" | "detail" | "regex";

/**
 * A user-configured magnet-search site template (rendered by the built-in
 * parser; nothing site-specific is compiled into the binary).
 */
export interface SiteTemplate {
  domain: string;
  /** Search URL template with `{query}` substituted at search time. */
  search: string;
  /** `table` (direct magnet links), `detail` (follow detail pages), or `regex`. */
  extract: ExtractStrategy;
  /** For `detail`: how many detail pages to follow. */
  max_pages?: number | null;
  /** For `regex`: the literal regex to apply. */
  pattern?: string | null;
}

export type TransferState =
  | "queued" | "resolving" | "downloading" | "paused" | "verifying"
  | "completed" | "postprocessing" | "ready" | "failed" | "cancelled" | "orphaned";

export type PostprocessState = "none" | "pending" | "running" | "completed" | "failed";

export interface TransferFile {
  index: string;
  path: string;
  size: number;
  selected: boolean;
  bytes_completed: number;
}

export interface Transfer {
  id: string;
  backend: Backend;
  source: string;
  display_name: string;
  state: TransferState;
  progress: number; // 0..1
  bytes_total: number | null;
  bytes_completed: number;
  download_rate: number | null; // bytes/s
  upload_rate: number | null;
  eta: number | null; // seconds
  destination: string;
  created_at: string;
  started_at: string | null;
  completed_at: string | null;
  error: string | null;
  files: TransferFile[];
  postprocess_state: PostprocessState;
  metadata: Record<string, unknown>;
}

export interface BackendStatus {
  backend: string;
  transfer_available: boolean;
  search_available: boolean;
  state: string;
}

export interface StatusResponse {
  version: string;
  uptime_secs: number;
  server_time: string;
  db: string;
  backends: BackendStatus[];
}

export interface AddTransferRequest {
  backend: Backend;
  source: string;
  destination?: string | null;
  display_name?: string | null;
}

export interface AddTransferResponse {
  transfer_id: string;
}

export interface SearchResult {
  result_id: string;
  search_id: string;
  username: string;
  path: string;
  filename: string;
  size: number | null;
  extension: string | null;
  bitrate: number | null;
  duration: number | null;
  queue_length: number | null;
  free_upload_slots: boolean | null;
  upload_speed: number | null;
  attributes: Record<string, unknown>;
  backend_metadata: Record<string, unknown>;
}

export interface AppEvent {
  kind: string;
  payload: Record<string, unknown>;
  ts: string;
}

export interface LibraryEntry {
  path: string;
  absolute_path: string;
  size: number | null;
  is_dir: boolean;
}
