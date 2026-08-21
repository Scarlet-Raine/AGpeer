import { useCallback, useEffect, useState } from "react";
import { formatBytes, getLibrary, openPath } from "../api/client";
import type { LibraryEntry } from "../api/types";

interface Node {
  name: string;
  path: string;
  abs: string;
  isDir: boolean;
  size: number | null;
  children: Node[];
}

function buildTree(entries: LibraryEntry[]): Node[] {
  const root: Node[] = [];
  const map = new Map<string, Node>();
  for (const e of entries) {
    const segments = e.path.split(/[\\/]/).filter(Boolean);
    if (segments.length === 0) continue;
    let level = root;
    let currentPath = "";
    for (let i = 0; i < segments.length; i++) {
      currentPath = currentPath ? currentPath + "/" + segments[i] : segments[i];
      let node = map.get(currentPath);
      if (!node) {
        node = {
          name: segments[i],
          path: currentPath,
          abs: e.absolute_path,
          isDir: i < segments.length - 1 || e.is_dir,
          size: i === segments.length - 1 && !e.is_dir ? e.size : null,
          children: [],
        };
        map.set(currentPath, node);
        level.push(node);
      }
      level = node.children;
    }
  }
  return root.sort((a, b) => Number(b.isDir) - Number(a.isDir) || a.name.localeCompare(b.name));
}

function TreeNode({ node, onNotice }: { node: Node; onNotice: (m: string) => void }) {
  const [open, setOpen] = useState(false);

  const reveal = async (abs: string) => {
    const ok = await openPath(abs);
    onNotice(ok ? `Opened ${abs}` : `Copied path: ${abs}`);
  };

  return (
    <div>
      <div
        className="muted"
        style={{ display: "flex", gap: 6, alignItems: "center", padding: "2px 0" }}
      >
        <span
          style={{ cursor: node.isDir ? "pointer" : "default", display: "flex", alignItems: "center", gap: 6, flex: 1 }}
          onClick={() => node.isDir && setOpen((o) => !o)}
        >
          <span>{node.isDir ? (open ? "▾" : "▸") : "·"}</span>
          <span style={{ color: "var(--text)" }}>{node.name}</span>
          {!node.isDir && node.size != null && <span>{formatBytes(node.size)}</span>}
        </span>
        {node.isDir && (
          <button
            className="btn"
            style={{ padding: "2px 8px", fontSize: 11 }}
            title="Open folder"
            onClick={(e) => {
              e.stopPropagation();
              void reveal(node.abs);
            }}
          >
            📂
          </button>
        )}
      </div>
      {open && node.children.map((c) => (
        <div key={c.path} style={{ paddingLeft: 18 }}>
          <TreeNode node={c} onNotice={onNotice} />
        </div>
      ))}
    </div>
  );
}

export default function Library() {
  const [tree, setTree] = useState<Node[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [loaded, setLoaded] = useState(false);

  const load = useCallback(async () => {
    try {
      const entries = await getLibrary();
      setTree(buildTree(entries));
      setLoaded(true);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setLoaded(true);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  return (
    <div>
      <h2>Library</h2>
      {error && <div className="muted" style={{ color: "var(--danger)" }}>{error}</div>}
      {notice && <div className="muted">{notice}</div>}
      {loaded && !error && tree.length === 0 && (
        <div className="card muted">The library is empty. Completed downloads are organized here (e.g. under TV Shows / Movies / Music).</div>
      )}
      {tree.map((n) => <TreeNode key={n.path} node={n} onNotice={setNotice} />)}
    </div>
  );
}
