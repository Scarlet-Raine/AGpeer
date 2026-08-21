import { ReactNode, useState } from "react";
import AppShell from "./components/AppShell";
import Home from "./components/Home";
import Downloads from "./components/Downloads";
import Completed from "./components/Completed";
import Search from "./components/Search";
import MagnetSearch from "./components/MagnetSearch";
import Library from "./components/Library";
import Queue from "./components/Queue";
import Settings from "./components/Settings";
import Agent from "./components/Agent";

type Screen = "home" | "downloads" | "completed" | "soulseek" | "magnet" | "library" | "queue" | "settings" | "agent";

export default function App() {
  const [screen, setScreen] = useState<Screen>("home");

  // All screens stay MOUNTED so their state (e.g. an in-progress Soulseek
  // search, its results, or the Downloads list) survives switching tabs; the
  // inactive ones are simply hidden instead of unmounted.
  const panels: Array<[Screen, ReactNode]> = [
    ["home", <Home />],
    ["downloads", <Downloads />],
    ["completed", <Completed />],
    ["soulseek", <Search />],
    ["magnet", <MagnetSearch />],
    ["library", <Library />],
    ["queue", <Queue />],
    ["settings", <Settings />],
    ["agent", <Agent />],
  ];

  return (
    <AppShell screen={screen} onNavigate={setScreen}>
      {panels.map(([id, node]) => (
        <div key={id} style={{ display: id === screen ? "block" : "none" }}>
          {node}
        </div>
      ))}
    </AppShell>
  );
}
