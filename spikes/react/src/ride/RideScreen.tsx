import { useEffect, useEffectEvent, useReducer, useRef, useState } from "react";
import type { Layout, Page } from "@neo/shared";
import { backend } from "../backend";
import { getStatus, subscribeStatus } from "../lib/live";
import { go } from "../lib/router";
import { layoutReducer, rowsOf } from "../layout/model";
import { LayoutEditor } from "../layout/LayoutEditor";
import { TopBar } from "./TopBar";
import { HelpOverlay } from "./HelpOverlay";
import { WidgetView } from "./widgets/WidgetView";
import { respondLatest } from "./actions";

function initialLayout(): Layout {
  const id = innerWidth < 1200 ? "tablet" : "laptop";
  const l = backend.layouts.get(id) ?? backend.layouts.list()[0];
  if (!l) throw new Error("no layouts");
  return l;
}

function pageFromHash(l: Layout): number {
  const want = new URLSearchParams(location.hash.split("?")[1] ?? "").get("page");
  const i = l.pages.findIndex((p) => p.id === want);
  return Math.max(0, i);
}

export function PageGrid({ page, rows }: { page: Page; rows: number }) {
  return (
    <div className="grid" style={{ gridTemplateRows: `repeat(${String(rows)}, minmax(0, 1fr))` }}>
      {page.widgets.map((w) => (
        <div key={w.id} className="cell" style={{ gridColumn: `${String(w.x + 1)} / span ${String(w.w)}`, gridRow: `${String(w.y + 1)} / span ${String(w.h)}` }}>
          <WidgetView widget={w} />
        </div>
      ))}
    </div>
  );
}

export function RideScreen() {
  const [layout, dispatch] = useReducer(layoutReducer, null, initialLayout);
  const [pageIdx, setPageIdx] = useState(() => pageFromHash(layout));
  const [autoPage, setAutoPage] = useState(() => !location.hash.includes("page="));
  const [editing, setEditing] = useState(() => location.hash.includes("edit=1"));
  const [help, setHelp] = useState(false);
  const page = layout.pages[Math.min(pageIdx, layout.pages.length - 1)] ?? layout.pages[0];
  const rows = rowsOf(layout);

  // Context-aware page switching: follow the ride phase (status events, not render-time state).
  useEffect(() => {
    if (!autoPage || editing) return;
    let last = "";
    const sync = () => {
      const ph = getStatus()?.phase;
      if (!ph || ph === last) return;
      last = ph;
      const i = layout.pages.findIndex((p) => p.autoPhase?.includes(ph));
      if (i >= 0) setPageIdx(i);
    };
    sync();
    return subscribeStatus(sync);
  }, [autoPage, editing, layout]);

  const turn = (d: number) => {
    setAutoPage(false);
    setPageIdx((i) => (i + d + layout.pages.length) % layout.pages.length);
  };

  const onKey = useEffectEvent((e: KeyboardEvent) => {
    if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement || e.target instanceof HTMLSelectElement) return;
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    const st = getStatus()?.state;
    switch (e.key) {
      case "ArrowRight":
        turn(1);
        break;
      case "ArrowLeft":
        turn(-1);
        break;
      case "+":
      case "=":
        backend.ride.adjust(5);
        break;
      case "-":
      case "_":
        backend.ride.adjust(-5);
        break;
      case "s":
      case "S":
        backend.ride.skipBlock();
        break;
      case " ":
        if (st === "riding") backend.ride.pause();
        else if (st === "paused") backend.ride.resume();
        else if (st === "idle") backend.ride.start();
        break;
      case "y":
      case "Y":
        respondLatest(true);
        break;
      case "n":
      case "N":
        respondLatest(false);
        break;
      case "a":
      case "A":
        setAutoPage((v) => !v);
        break;
      case "e":
      case "E":
        setEditing((v) => !v);
        break;
      case "1":
        backend.ride.setSpeed(1);
        break;
      case "2":
        backend.ride.setSpeed(10);
        break;
      case "3":
        backend.ride.setSpeed(60);
        break;
      case "?":
        setHelp((v) => !v);
        break;
      case "Escape":
        setHelp(false);
        break;
      default:
        return;
    }
    e.preventDefault();
  });
  useEffect(() => {
    const h = (e: KeyboardEvent) => {
      onKey(e);
    };
    addEventListener("keydown", h);
    return () => {
      removeEventListener("keydown", h);
    };
  }, []);

  // swipe between pages
  const swipe = useRef<{ x: number; y: number } | null>(null);

  if (!page) return null;
  return (
    <div className="ride" data-page={page.id}>
      <TopBar editing={editing} onEdit={() => { setEditing(!editing); }} onHelp={() => { setHelp(true); }} onExit={() => { backend.ride.pause(); go("review"); }} />
      {editing ? (
        <LayoutEditor layout={layout} dispatch={dispatch} pageIdx={pageIdx} setPageIdx={setPageIdx} onDone={() => { setEditing(false); }} />
      ) : (
        <main
          className="ride-main"
          onPointerDown={(e) => {
            if (e.pointerType !== "mouse") swipe.current = { x: e.clientX, y: e.clientY };
          }}
          onPointerUp={(e) => {
            const s = swipe.current;
            swipe.current = null;
            if (!s) return;
            const dx = e.clientX - s.x;
            if (Math.abs(dx) > 80 && Math.abs(dx) > 2 * Math.abs(e.clientY - s.y)) turn(dx < 0 ? 1 : -1);
          }}
        >
          <PageGrid page={page} rows={rows} />
        </main>
      )}
      <nav className="pager" aria-label="pages">
        {layout.pages.map((p, i) => (
          <button type="button" key={p.id} className={i === pageIdx ? "on" : ""} onClick={() => { setAutoPage(false); setPageIdx(i); }}>
            <span className="dot" /> {p.name}
          </button>
        ))}
        <label className="check small" title="Switch page by ride phase (A)">
          <input type="checkbox" checked={autoPage} onChange={(e) => { setAutoPage(e.target.checked); }} /> auto page
        </label>
      </nav>
      {help && <HelpOverlay onClose={() => { setHelp(false); }} />}
    </div>
  );
}
