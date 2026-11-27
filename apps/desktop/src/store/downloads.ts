import { create } from "zustand";
import { api, events, type DownloadStatus, type Progress } from "../lib/ipc";

export type Filter = "all" | "active" | "done" | "failed";

const HISTORY = 60;

interface DownloadsState {
  items: Record<number, Progress>;
  loaded: boolean;
  filter: Filter;
  /** Total throughput, one sample per second, newest last. */
  history: number[];
  load: () => Promise<void>;
  upsert: (p: Progress) => void;
  upsertMany: (ps: Progress[]) => void;
  setStatus: (id: number, status: DownloadStatus) => void;
  remove: (id: number) => void;
  setFilter: (f: Filter) => void;
  sample: () => void;
  counts: () => Record<Filter, number>;
}

export function matches(p: Progress, f: Filter): boolean {
  const s = p.status.state;
  switch (f) {
    case "all":
      return true;
    case "active":
      return s === "queued" || s === "probing" || s === "downloading" || s === "processing" || s === "paused";
    case "done":
      return s === "completed";
    case "failed":
      return s === "failed" || s === "cancelled";
  }
}

export const useDownloads = create<DownloadsState>((set, get) => ({
  items: {},
  loaded: false,
  filter: "all",
  history: Array.from({ length: HISTORY }, () => 0),
  load: async () => {
    const list = await api.list();
    set({ items: Object.fromEntries(list.map((p) => [p.id, p])), loaded: true });
  },
  upsert: (p) => set((s) => ({ items: { ...s.items, [p.id]: p } })),
  upsertMany: (ps) =>
    set((s) => {
      const items = { ...s.items };
      for (const p of ps) items[p.id] = p;
      return { items };
    }),
  setStatus: (id, status) =>
    set((s) => {
      const cur = s.items[id];
      if (!cur) return s;
      const keepSpeed = status.state === "downloading";
      return { items: { ...s.items, [id]: { ...cur, status, speed_bps: keepSpeed ? cur.speed_bps : 0, eta_secs: keepSpeed ? cur.eta_secs : null } } };
    }),
  remove: (id) =>
    set((s) => {
      const items = { ...s.items };
      delete items[id];
      return { items };
    }),
  setFilter: (filter) => set({ filter }),
  sample: () => {
    const total = Object.values(get().items).reduce(
      (a, p) => a + (p.status.state === "downloading" ? p.speed_bps : 0),
      0,
    );
    set((s) => ({ history: [...s.history.slice(-(HISTORY - 1)), total] }));
  },
  counts: () => countsOf(get().items),
}));

export function countsOf(items: Record<number, Progress>): Record<Filter, number> {
  const c: Record<Filter, number> = { all: 0, active: 0, done: 0, failed: 0 };
  for (const p of Object.values(items)) {
    c.all++;
    if (matches(p, "active")) c.active++;
    else if (matches(p, "done")) c.done++;
    else if (matches(p, "failed")) c.failed++;
  }
  return c;
}

/** Wire engine events into the store once; returns a cleanup fn. */
export function subscribeDownloads(): () => void {
  const st = useDownloads.getState();
  const unlisteners: Promise<() => void>[] = [
    events.onProgress(st.upsertMany),
    events.onAdded(st.upsert),
    events.onStatus(st.setStatus),
    events.onRemoved(st.remove),
  ];
  void st.load();
  const timer = setInterval(() => useDownloads.getState().sample(), 1000);
  return () => {
    clearInterval(timer);
    for (const u of unlisteners) void u.then((f) => f());
  };
}

const ORDER: Record<DownloadStatus["state"], number> = {
  downloading: 0,
  probing: 0,
  processing: 0,
  queued: 1,
  paused: 2,
  failed: 3,
  cancelled: 3,
  completed: 4,
};

/** Active work first, then waiting, then history by recency. */
export function sortedDownloads(items: Record<number, Progress>): Progress[] {
  return Object.values(items).sort((a, b) => {
    const d = ORDER[a.status.state] - ORDER[b.status.state];
    if (d !== 0) return d;
    return (b.completed_at ?? b.created_at) - (a.completed_at ?? a.created_at);
  });
}
