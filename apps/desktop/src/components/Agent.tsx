import { useEffect, useState } from "react";
import { API_BASE, getApiToken } from "../api/client";

const ENDPOINTS: Array<[string, string, string]> = [
  ["GET", "/api/v1/status", "Core status, uptime, backends"],
  ["GET", "/api/v1/backends", "Backend capability and state"],
  ["GET", "/api/v1/transfers", "List transfers"],
  ["POST", "/api/v1/transfers", "Add a transfer"],
  ["GET", "/api/v1/transfers/{id}", "Get a transfer"],
  ["POST", "/api/v1/transfers/{id}/pause", "Pause a transfer"],
  ["POST", "/api/v1/transfers/{id}/resume", "Resume a transfer"],
  ["POST", "/api/v1/transfers/{id}/cancel", "Cancel a transfer"],
  ["DELETE", "/api/v1/transfers/{id}", "Remove a transfer"],
  ["GET", "/api/v1/searches", "List searches"],
  ["POST", "/api/v1/searches", "Start a Soulseek search"],
  ["GET", "/api/v1/searches/{id}/results", "Get search results"],
  ["POST", "/api/v1/searches/{id}/results/{rid}/download", "Download a result"],
  ["GET", "/api/v1/settings", "List runtime settings"],
  ["PUT", "/api/v1/settings", "Set runtime settings"],
  ["GET", "/api/v1/postprocess", "List post-processing jobs"],
  ["GET", "/api/v1/library", "List the organized library"],
];

export default function Agent() {
  const [token, setToken] = useState<string | null>(null);
  const [revealed, setRevealed] = useState(false);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    getApiToken().then((t) => setToken(t));
  }, []);

  async function copyToken() {
    if (!token) return;
    try {
      await navigator.clipboard.writeText(token);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      /* clipboard unavailable */
    }
  }

  const masked = token ? token.slice(0, 6) + "…" + token.slice(-4) : "";

  return (
    <div>
      <h2>Agent / API</h2>

      <div className="card">
        <h3>Credentials</h3>
        <p className="muted">
          API base: <code>{API_BASE}</code>
        </p>
        <div className="form-row">
          <code style={{ flex: 1 }}>
            Authorization: Bearer {revealed && token ? token : masked || "(no token)"}
          </code>
          <button className="btn" onClick={() => setRevealed((r) => !r)} disabled={!token}>
            {revealed ? "Hide" : "Reveal"}
          </button>
          <button className="btn" onClick={copyToken} disabled={!token}>
            {copied ? "Copied" : "Copy"}
          </button>
        </div>
        <p className="muted">
          Every request — including loopback — must present this bearer token. Never share it.
        </p>
      </div>

      <div className="card">
        <h3>API documentation</h3>
        <p className="muted">
          Interactive Swagger UI:{" "}
          <a href={`${API_BASE}/api/v1/docs`} target="_blank" rel="noreferrer">
            {API_BASE}/api/v1/docs
          </a>
        </p>
        <p className="muted">
          OpenAPI JSON:{" "}
          <a href={`${API_BASE}/api/v1/docs/openapi.json`} target="_blank" rel="noreferrer">
            {API_BASE}/api/v1/docs/openapi.json
          </a>
        </p>
      </div>

      <div className="card">
        <h3>Endpoints</h3>
        <table style={{ width: "100%", borderCollapse: "collapse" }}>
          <thead>
            <tr className="muted" style={{ textAlign: "left" }}>
              <th>Method</th>
              <th>Path</th>
              <th>Description</th>
            </tr>
          </thead>
          <tbody>
            {ENDPOINTS.map(([method, path, desc]) => (
              <tr key={method + path} style={{ borderTop: "1px solid var(--border)" }}>
                <td className="muted">{method}</td>
                <td><code>{path}</code></td>
                <td className="muted">{desc}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
