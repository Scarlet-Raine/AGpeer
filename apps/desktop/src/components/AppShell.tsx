import { ReactNode, useEffect, useState } from "react";
import { getStatus } from "../api/client";
import { subscribeEvents, ConnectionStatus } from "../api/events";

export type Screen = "home" | "downloads" | "completed" | "soulseek" | "magnet" | "library" | "queue" | "settings" | "agent";

interface Props {
  screen: Screen;
  onNavigate: (s: Screen) => void;
  children: ReactNode;
}

const NAV_ITEMS = [
  { id: "home", label: "Home" },
  { id: "downloads", label: "Downloads" },
  { id: "completed", label: "Completed" },
  { id: "soulseek", label: "Soulseek Search" },
  { id: "magnet", label: "Magnet Search" },
  { id: "library", label: "Library" },
  { id: "queue", label: "Queue" },
  { id: "settings", label: "Settings" },
  { id: "agent", label: "Agent / API" },
] as const;

export default function AppShell({ screen, onNavigate, children }: Props) {
  const [conn, setConn] = useState<ConnectionStatus>("connecting");
  const [version, setVersion] = useState<string>("");

  useEffect(() => {
    const stop = subscribeEvents(() => {}, setConn);
    return stop;
  }, []);

  useEffect(() => {
    getStatus()
      .then((s) => setVersion(s.version))
      .catch((e) => console.warn("[agpeer] status check failed", e));
  }, []);

  return (
    <div className="app-shell">
      <nav className="sidebar">
        {NAV_ITEMS.map((item) => (
          <a
            key={item.id}
            className={screen === item.id ? "active" : ""}
            onClick={() => onNavigate(item.id)}
          >
            {item.label}
          </a>
        ))}
      </nav>
      <main className="main">
        <div className="topbar">
          <h1>agpeer</h1>
          <div style={{ display: "flex", gap: 12, alignItems: "center" }}>
            <span
              className={`status-dot ${conn === "connected" ? "ok" : "err"}`}
              title={`core: ${conn}`}
            />
            {version && <span className="muted">v{version}</span>}
          </div>
        </div>
        {children}
      </main>
    </div>
  );
}
