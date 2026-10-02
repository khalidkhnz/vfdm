import type { SegmentProgress } from "../lib/ipc";

const COLORS = ["bg-sky-400", "bg-emerald-400", "bg-amber-400", "bg-fuchsia-400", "bg-rose-400", "bg-cyan-400", "bg-lime-400", "bg-violet-400"];

interface Props {
  segments: SegmentProgress[];
  total: number | null;
  done: boolean;
}

/** IDM-style stripes: each segment paints its own slice at its byte offset. */
export default function SegmentBar({ segments, total, done }: Props) {
  if (done) return <div className="h-1.5 w-full rounded-full bg-emerald-500" />;
  if (!total || segments.length === 0) {
    return <div className="h-1.5 w-full overflow-hidden rounded-full bg-neutral-800" />;
  }
  return (
    <div className="relative h-1.5 w-full overflow-hidden rounded-full bg-neutral-800">
      {segments.map((s, i) => {
        const left = (s.start / total) * 100;
        const width = (s.downloaded / total) * 100;
        if (width <= 0) return null;
        return (
          <div
            key={`${s.start}-${i}`}
            className={`absolute top-0 h-full ${COLORS[i % COLORS.length]}`}
            style={{ left: `${left}%`, width: `${Math.max(width, 0.15)}%` }}
          />
        );
      })}
    </div>
  );
}
