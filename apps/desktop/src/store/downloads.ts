import { create } from "zustand";
import { api, events, type DownloadStatus, type Progress } from "../lib/ipc";

interface DownloadsState {
  items: Record<number, Progress>;
  loaded: boolean;
  load: () => Promise<void>;
  upsert: (p: Progress) => void;
  upsertMany: (ps: Progress[]) => void;
  setStatus: (id: number, status: DownloadStatus) => void;
  remove: (id: number) => void;
}

export const useDownloads = create<DownloadsState>((set) => ({
  items: {},
  loaded: false,
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
      const speed_bps = status.state === "downloading" ? cur.speed_bps : 0;
      return { items: { ...s.items, [id]: { ...cur, status, speed_bps, eta_secs: null } } };
    }),
  remove: (id) =>
    set((s) => {
      const items = { ...s.items };
      delete items[id];
      return { items };
    }),
}));

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
  return () => {
    for (const u of unlisteners) void u.then((f) => f());
  };
}

export function sortedDownloads(items: Record<number, Progress>): Progress[] {
  return Object.values(items).sort((a, b) => b.created_at - a.created_at);
}
