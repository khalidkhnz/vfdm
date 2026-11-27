import { useMemo } from "react";
import { useDownloads } from "../store/downloads";

const UNITS = ["B/s", "KB/s", "MB/s", "GB/s"];

function splitRate(bps: number): { value: string; unit: string } {
  let v = bps;
  let i = 0;
  while (v >= 1000 && i < UNITS.length - 1) {
    v /= 1024;
    i++;
  }
  return { value: v >= 100 ? v.toFixed(0) : v >= 10 ? v.toFixed(1) : v.toFixed(2), unit: UNITS[i] };
}

export default function Throughput() {
  const history = useDownloads((s) => s.history);
  const items = useDownloads((s) => s.items);
  const current = history[history.length - 1] ?? 0;
  const rate = splitRate(current);

  const counts = useMemo(() => {
    let active = 0;
    let queued = 0;
    for (const p of Object.values(items)) {
      const st = p.status.state;
      if (st === "downloading" || st === "probing" || st === "processing") active++;
      else if (st === "queued") queued++;
    }
    return { active, queued };
  }, [items]);

  const path = useMemo(() => {
    const w = 160;
    const h = 36;
    const max = Math.max(1, ...history);
    const n = history.length;
    if (n < 2) return { line: "", area: "" };
    const pts = history.map((v, i) => [(i / (n - 1)) * w, h - (v / max) * (h - 3) - 1]);
    const line = pts.map(([x, y], i) => `${i ? "L" : "M"}${x.toFixed(1)},${y.toFixed(1)}`).join(" ");
    return { line, area: `${line} L${w},${h} L0,${h} Z` };
  }, [history]);

  return (
    <div className="flex items-end gap-5">
      <div className="flex items-baseline gap-1.5 leading-none">
        <span className="font-mono text-[34px] font-medium tabular-nums tracking-tight text-ink">{rate.value}</span>
        <span className="font-mono text-[13px] text-mute">{rate.unit}</span>
      </div>
      <svg width={160} height={36} className="shrink-0" aria-label="Throughput over the last minute">
        {path.area && <path d={path.area} fill="var(--color-accent)" opacity={0.12} />}
        {path.line && <path d={path.line} fill="none" stroke="var(--color-accent)" strokeWidth={1.5} strokeLinejoin="round" />}
      </svg>
      <div className="pb-1 font-mono text-[12.5px] tabular-nums text-mute">
        {counts.active} active
        {counts.queued > 0 && <span className="text-dim">{"  "}{counts.queued} queued</span>}
      </div>
    </div>
  );
}
