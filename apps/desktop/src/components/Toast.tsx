import { create } from "zustand";
import { Download, X } from "lucide-react";

interface Toast {
  id: number;
  title: string;
  body?: string;
  kind: "info" | "error";
}

interface ToastState {
  toasts: Toast[];
  push: (t: Omit<Toast, "id">) => void;
  dismiss: (id: number) => void;
}

let seq = 1;

export const useToasts = create<ToastState>((set) => ({
  toasts: [],
  push: (t) => {
    const id = seq++;
    set((s) => ({ toasts: [...s.toasts.slice(-3), { ...t, id }] }));
    setTimeout(() => set((s) => ({ toasts: s.toasts.filter((x) => x.id !== id) })), 5000);
  },
  dismiss: (id) => set((s) => ({ toasts: s.toasts.filter((x) => x.id !== id) })),
}));

export default function Toasts() {
  const toasts = useToasts((s) => s.toasts);
  const dismiss = useToasts((s) => s.dismiss);
  if (toasts.length === 0) return null;
  return (
    <div className="pointer-events-none fixed right-4 bottom-4 z-40 flex w-80 flex-col gap-2" aria-live="polite">
      {toasts.map((t) => (
        <div
          key={t.id}
          className={`toast-in pointer-events-auto flex items-start gap-3 rounded-lg border bg-surface px-3.5 py-3 shadow-[0_12px_32px_-12px_rgba(0,0,0,.8)] ${
            t.kind === "error" ? "border-error/40" : "border-line"
          }`}
        >
          <span className={`mt-0.5 ${t.kind === "error" ? "text-error" : "text-accent"}`}>
            <Download size={15} />
          </span>
          <div className="min-w-0 flex-1">
            <div className="text-[13px] font-medium">{t.title}</div>
            {t.body && <div className="mt-0.5 truncate text-[12px] text-mute">{t.body}</div>}
          </div>
          <button onClick={() => dismiss(t.id)} className="text-dim hover:text-ink" aria-label="Dismiss">
            <X size={14} />
          </button>
        </div>
      ))}
    </div>
  );
}
