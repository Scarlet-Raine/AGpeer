import { FormEvent, useCallback, useEffect, useState } from "react";
import { getSettings, putSettings, deleteSetting } from "../api/client";
import type { SiteTemplate, ExtractStrategy } from "../api/types";

const KiB = 1024;

function num(v: unknown): string {
  if (typeof v === "number") return String(v);
  if (typeof v === "string") return v;
  return "";
}

export default function Settings() {
  const [settings, setSettings] = useState<Record<string, unknown>>({});
  const [dlRate, setDlRate] = useState("");
  const [ulRate, setUlRate] = useState("");
  const [maxActive, setMaxActive] = useState("");
  const [notice, setNotice] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [newKey, setNewKey] = useState("");
  const [newValue, setNewValue] = useState("");
  const [saving, setSaving] = useState(false);

  const magnetEnabled = settings["hook_search.enabled"] === true;
  const domains: string[] = Array.isArray(settings["hook_search.domains"])
    ? (settings["hook_search.domains"] as string[])
    : [];
  const [domainInput, setDomainInput] = useState("");
  const sites: SiteTemplate[] = Array.isArray(settings["hook_search.sites"])
    ? (settings["hook_search.sites"] as SiteTemplate[])
    : [];
  const [siteForm, setSiteForm] = useState<SiteTemplate>({
    domain: "",
    search: "",
    extract: "table",
    max_pages: null,
    pattern: null,
  });

  async function onToggleMagnet() {
    setSaving(true);
    setError(null);
    setNotice(null);
    try {
      await putSettings({ "hook_search.enabled": !magnetEnabled });
      setNotice(magnetEnabled ? "Magnet search disabled." : "Magnet search enabled.");
      await load();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  }

  async function onAddDomains(e: FormEvent) {
    e.preventDefault();
    const parts = domainInput
      .split(/[\s,;]+/)
      .map((s) => s.trim())
      .filter((s) => s.length > 0);
    const merged = Array.from(new Set([...domains, ...parts]));
    if (merged.length === 0) return;
    setSaving(true);
    setError(null);
    setNotice(null);
    try {
      await putSettings({ "hook_search.domains": merged });
      setDomainInput("");
      setNotice(`Domains updated (${merged.length} total).`);
      await load();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(false);
    }
  }

  async function onRemoveDomain(domain: string) {
    const merged = domains.filter((d) => d !== domain);
    setSaving(true);
    setError(null);
    setNotice(null);
    try {
      await putSettings({ "hook_search.domains": merged });
      await load();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(false);
    }
  }

  async function onAddSite(e: FormEvent) {
    e.preventDefault();
    const domain = siteForm.domain.trim();
    const search = siteForm.search.trim();
    if (!domain || !search) {
      setError("Enter a site name and a search URL.");
      return;
    }
    if (!search.includes("{query}")) {
      setError("The search URL must contain a {query} token.");
      return;
    }
    if (siteForm.extract === "regex" && !siteForm.pattern?.trim()) {
      setError("The regex strategy needs a pattern.");
      return;
    }
    const entry: SiteTemplate = {
      domain,
      search,
      extract: siteForm.extract as ExtractStrategy,
      max_pages:
        siteForm.extract === "detail" && siteForm.max_pages ? Number(siteForm.max_pages) : null,
      pattern: siteForm.extract === "regex" ? siteForm.pattern : null,
    };
    // Keyed on `domain` alone so editing an existing site's search URL (or
    // strategy) replaces it instead of accumulating stale duplicate rows.
    const merged = [...sites.filter((s) => s.domain !== entry.domain), entry];
    setSaving(true);
    setError(null);
    setNotice(null);
    try {
      await putSettings({ "hook_search.sites": merged });
      setSiteForm({ domain: "", search: "", extract: "table", max_pages: null, pattern: null });
      setNotice(`Site template saved (${merged.length} total).`);
      await load();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(false);
    }
  }

  function onRemoveSite(index: number) {
    const merged = sites.filter((_, i) => i !== index);
    setSaving(true);
    setError(null);
    setNotice(null);
    putSettings({ "hook_search.sites": merged })
      .then(load)
      .catch((err) => setError(err instanceof Error ? err.message : String(err)))
      .finally(() => setSaving(false));
  }

  const load = useCallback(async () => {
    try {
      const all = await getSettings();
      setSettings(all);
      const dl = all["queue.download_rate_limit"];
      const ul = all["queue.upload_rate_limit"];
      const ma = all["queue.max_active"];
      if (dl != null) {
        const b = typeof dl === "number" ? dl : parseFloat(String(dl));
        setDlRate(Number.isFinite(b) ? String(Math.round(b / KiB)) : "");
      }
      if (ul != null) {
        const b = typeof ul === "number" ? ul : parseFloat(String(ul));
        setUlRate(Number.isFinite(b) ? String(Math.round(b / KiB)) : "");
      }
      if (ma != null) setMaxActive(num(ma));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  async function onSaveQueue(e: FormEvent) {
    e.preventDefault();
    setSaving(true);
    setError(null);
    setNotice(null);
    try {
      const map: Record<string, unknown> = {};
      if (dlRate.trim() !== "") map["queue.download_rate_limit"] = Math.round(parseFloat(dlRate) || 0) * KiB;
      if (ulRate.trim() !== "") map["queue.upload_rate_limit"] = Math.round(parseFloat(ulRate) || 0) * KiB;
      if (maxActive.trim() !== "") map["queue.max_active"] = Math.max(1, Math.round(parseFloat(maxActive) || 1));
      if (Object.keys(map).length === 0) {
        throw new Error("Enter at least one value");
      }
      await putSettings(map);
      setNotice("Queue settings saved. Torrent rate limits are applied on the next core start.");
      await load();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  }

  async function onRemoveKey(key: string) {
    try {
      await deleteSetting(key);
      await load();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  async function onAddKey(e: FormEvent) {
    e.preventDefault();
    const key = newKey.trim();
    if (!key) return;
    setError(null);
    setNotice(null);
    try {
      const raw: string = newValue.trim();
      let value: unknown = raw;
      if (raw !== "") {
        const lower = raw.toLowerCase();
        if (lower === "true" || lower === "false") value = lower === "true";
        else if (/^-?\d+\.?\d*$/.test(raw)) value = parseFloat(raw);
      }
      await putSettings({ [key]: value });
      setNewKey("");
      setNewValue("");
      await load();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  const entries = Object.entries(settings).sort(([a], [b]) => a.localeCompare(b));

  return (
    <div>
      <h2>Settings</h2>
      {error && <div className="muted" style={{ color: "var(--danger)" }}>{error}</div>}
      {notice && <div className="muted" style={{ color: "var(--success)" }}>{notice}</div>}

      <div className="card">
        <h3>Queue</h3>
        <form className="form-row" onSubmit={onSaveQueue}>
          <label className="muted" style={{ display: "flex", flexDirection: "column", gap: 4 }}>
            Torrent download limit (KiB/s)
            <input type="number" min="0" value={dlRate} onChange={(e) => setDlRate(e.target.value)} style={{ width: 180 }} />
          </label>
          <label className="muted" style={{ display: "flex", flexDirection: "column", gap: 4 }}>
            Torrent upload limit (KiB/s)
            <input type="number" min="0" value={ulRate} onChange={(e) => setUlRate(e.target.value)} style={{ width: 180 }} />
          </label>
          <label className="muted" style={{ display: "flex", flexDirection: "column", gap: 4 }}>
            Max active downloads
            <input type="number" min="1" value={maxActive} onChange={(e) => setMaxActive(e.target.value)} style={{ width: 120 }} />
          </label>
          <button type="submit" className="btn" disabled={saving}>
            {saving ? "Saving…" : "Save"}
          </button>
        </form>
        <p className="muted">Rate limits are bytes/sec on the server; they are read at core startup (librqbit applies them when the session is created).</p>
      </div>

      <div className="card">
        <h3>Magnet search</h3>
        <div style={{ display: "flex", gap: 16, alignItems: "center" }}>
          <label className="muted" style={{ display: "flex", gap: 6, alignItems: "center" }}>
            <input type="checkbox" checked={magnetEnabled} onChange={onToggleMagnet} disabled={saving} />
            Enabled
          </label>
          <button className="btn" onClick={onToggleMagnet} disabled={saving}>
            {saving ? "Saving…" : magnetEnabled ? "Disable" : "Enable"}
          </button>
        </div>
        <p className="muted">
          Enables magnet search (the built-in engine/site-template search, or
          your configured <code>[hook_search].command</code> override). Downloads
          pulled from results use the torrent backend.
        </p>
      </div>

      <div className="card">
        <h3>Magnet search domains</h3>
        <p className="muted">
          The sites/indexes your magnet-search command should query. These are handed to the hook
          command (replacing a <code>{domains}</code> token, or appended as extra arguments) at
          search time; what the command does with them is up to you.
        </p>
        {domains.length === 0 ? (
          <p className="muted">No domains configured.</p>
        ) : (
          domains.map((d) => (
            <div key={d} style={{ display: "flex", gap: 8, alignItems: "center", marginBottom: 6 }}>
              <code style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                {d}
              </code>
              <button className="btn danger" onClick={() => onRemoveDomain(d)} disabled={saving}>
                Remove
              </button>
            </div>
          ))
        )}
        <form className="form-row" onSubmit={onAddDomains} style={{ marginTop: 12 }}>
          <input
            placeholder="index-1.example, index-2.example"
            value={domainInput}
            onChange={(e) => setDomainInput(e.target.value)}
            style={{ flex: 1 }}
          />
          <button type="submit" className="btn" disabled={saving || !domainInput.trim()}>
            Add
          </button>
        </form>
      </div>

      <div className="card">
        <h3>Magnet search sites</h3>
        <p className="muted">
          Optional site templates for the built-in search (used when no{" "}
          <code>[hook_search].command</code> is configured). Each template runs{" "}
          <code>{`{query}`}</code> against a site URL you control;{" "}
          <code>table</code> takes direct magnet links, <code>detail</code>{" "}
          follows the top detail pages for each page's first magnet, and{" "}
          <code>regex</code> applies your pattern. Nothing site-specific is
          compiled into the binary.
        </p>
        {sites.length === 0 ? (
          <p className="muted">No site templates configured.</p>
        ) : (
          sites.map((s, i) => (
            <div
              key={`${s.domain}-${s.search}-${i}`}
              style={{ display: "flex", gap: 8, alignItems: "center", marginBottom: 6 }}
            >
              <code style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                {s.domain} ({s.extract}) — {s.search}
              </code>
              <button className="btn danger" onClick={() => onRemoveSite(i)} disabled={saving}>
                Remove
              </button>
            </div>
          ))
        )}
        <form className="form-row" onSubmit={onAddSite} style={{ marginTop: 12 }}>
          <input
            placeholder="name (e.g. index)"
            value={siteForm.domain}
            onChange={(e) => setSiteForm({ ...siteForm, domain: e.target.value })}
            style={{ width: 140 }}
          />
          <input
            placeholder="https://site.example/?q={query}"
            value={siteForm.search}
            onChange={(e) => setSiteForm({ ...siteForm, search: e.target.value })}
            style={{ flex: 1 }}
          />
          <select
            value={siteForm.extract}
            onChange={(e) =>
              setSiteForm({ ...siteForm, extract: e.target.value as ExtractStrategy })
            }
            style={{ width: 110 }}
          >
            <option value="table">table</option>
            <option value="detail">detail</option>
            <option value="regex">regex</option>
          </select>
          {siteForm.extract === "detail" && (
            <input
              type="number"
              min="1"
              max="10"
              placeholder="max pages"
              value={siteForm.max_pages ?? ""}
              onChange={(e) =>
                setSiteForm({
                  ...siteForm,
                  max_pages: e.target.value ? Number(e.target.value) : null,
                })
              }
              style={{ width: 90 }}
            />
          )}
          {siteForm.extract === "regex" && (
            <input
              placeholder="magnet pattern regex"
              value={siteForm.pattern ?? ""}
              onChange={(e) => setSiteForm({ ...siteForm, pattern: e.target.value })}
              style={{ flex: 1 }}
            />
          )}
          <button type="submit" className="btn" disabled={saving}>
            Add site
          </button>
        </form>
      </div>

      <div className="card">
        <h3>All settings</h3>
        {entries.length === 0 && <p className="muted">No settings stored yet.</p>}
        {entries.map(([key, value]) => (
          <div key={key} style={{ display: "flex", gap: 8, alignItems: "center", marginBottom: 6 }}>
            <code style={{ flex: 1 }}>{key} = {JSON.stringify(value)}</code>
            <button className="btn danger" onClick={() => onRemoveKey(key)}>Remove</button>
          </div>
        ))}
        <form className="form-row" onSubmit={onAddKey} style={{ marginTop: 12 }}>
          <input placeholder="key" value={newKey} onChange={(e) => setNewKey(e.target.value)} style={{ width: 220 }} />
          <input placeholder="value (JSON-ish)" value={newValue} onChange={(e) => setNewValue(e.target.value)} style={{ flex: 1 }} />
          <button type="submit" className="btn">Add</button>
        </form>
      </div>
    </div>
  );
}
