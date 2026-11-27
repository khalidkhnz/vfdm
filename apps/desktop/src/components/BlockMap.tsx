import { useEffect, useMemo, useRef, useState } from "react";
import type { Progress } from "../lib/ipc";

/**
 * The file's byte space (or a stream's segment list) as a row of cells.
 * Each cell shows how much of its slice has landed and which worker owns
 * it; cells under an active write head shimmer. This is the one place the
 * UI spends its motion budget.
 */

const CELL = 9;
const GAP = 2;

interface Cell {
  fill: number;
  hue: number;
  active: boolean;
  state: "pending" | "partial" | "done";
}

function hueFor(index: number, count: number): number {
  const t = count <= 1 ? 0 : index / (count - 1);
  return 222 - t * 36;
}

function fileCells(p: Progress, n: number): Cell[] {
  const total = p.total ?? 0;
  if (total <= 0) {
    return Array.from({ length: n }, () => ({ fill: 0, hue: 222, hue2: 0, active: false, state: "pending" as const }));
  }
  const size = total / n;
  const cells: Cell[] = [];
  for (let i = 0; i < n; i++) {
    const a = i * size;
    const b = (i + 1) * size;
    let covered = 0;
    let owner = -1;
    let ownerCover = 0;
    let active = false;
    p.segments.forEach((s, idx) => {
      const sa = s.start;
      const sb = s.start + s.downloaded;
      const lo = Math.max(a, sa);
      const hi = Math.min(b, sb);
      if (hi > lo) {
        covered += hi - lo;
        if (hi - lo > ownerCover) {
          ownerCover = hi - lo;
          owner = idx;
        }
      }
      const head = sb;
      if (s.downloaded < s.end - s.start + 1 && head >= a && head < b) active = true;
    });
    const fill = Math.min(1, covered / size);
    cells.push({
      fill,
      hue: hueFor(Math.max(owner, 0), p.segments.length),
      active: active && p.status.state === "downloading",
      state: fill >= 0.999 ? "done" : fill > 0 ? "partial" : "pending",
    });
  }
  return cells;
}

function streamCells(p: Progress, n: number): Cell[] {
  const count = p.segment_count ?? 0;
  const done = p.segments_done ?? 0;
  const ready = new Set(p.segments.filter((s) => s.start === s.end && s.start >= done).map((s) => s.start));
  const per = count / n;
  const cells: Cell[] = [];
  for (let i = 0; i < n; i++) {
    const a = i * per;
    const b = (i + 1) * per;
    const doneIn = Math.max(0, Math.min(b, done) - a);
    let readyIn = 0;
    for (const r of ready) if (r >= a && r < b) readyIn += 1;
    const fill = per > 0 ? Math.min(1, (doneIn + readyIn * 0.5) / per) : 0;
    const active = done >= a && done < b && p.status.state === "downloading";
    cells.push({
      fill,
      hue: 222 - (i / Math.max(1, n - 1)) * 36,
      active,
      state: fill >= 0.999 ? "done" : fill > 0 ? "partial" : "pending",
    });
  }
  return cells;
}

export default function BlockMap({ p }: { p: Progress }) {
  const ref = useRef<HTMLDivElement>(null);
  const [n, setN] = useState(64);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const ro = new ResizeObserver(([e]) => {
      const w = e.contentRect.width;
      setN(Math.max(24, Math.min(140, Math.floor((w + GAP) / (CELL + GAP)))));
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const cells = useMemo(
    () => (p.kind === "hls" || p.kind === "dash" ? streamCells(p, n) : fileCells(p, n)),
    [p, n],
  );
  const paused = p.status.state === "paused" || p.status.state === "queued";

  return (
    <div ref={ref} className="blockmap flex w-full" style={{ gap: GAP, height: CELL }} aria-hidden>
      {cells.map((c, i) => (
        <span
          key={i}
          className={`cell ${c.active ? "cell-active" : ""} ${paused ? "cell-paused" : ""}`}
          style={
            {
              width: CELL,
              "--fill": c.fill,
              "--h": c.hue,
            } as React.CSSProperties
          }
        />
      ))}
    </div>
  );
}
