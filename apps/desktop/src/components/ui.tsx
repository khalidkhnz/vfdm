import { useEffect, type ReactNode } from "react";
import { X } from "lucide-react";

type Variant = "primary" | "ghost" | "danger" | "outline";

const VARIANT: Record<Variant, string> = {
  primary: "bg-accent text-bg hover:bg-accent-hi",
  ghost: "text-mute hover:text-ink hover:bg-raised",
  danger: "text-error hover:bg-error/10",
  outline: "border border-line text-ink hover:bg-raised",
};

export function Button({
  variant = "outline",
  className = "",
  children,
  ...rest
}: React.ButtonHTMLAttributes<HTMLButtonElement> & { variant?: Variant }) {
  return (
    <button
      {...rest}
      className={`inline-flex h-8 items-center gap-1.5 rounded-md px-3 text-[13px] font-medium transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent disabled:cursor-default disabled:opacity-40 ${VARIANT[variant]} ${className}`}
    >
      {children}
    </button>
  );
}

export function IconButton({
  label,
  className = "",
  children,
  danger,
  ...rest
}: React.ButtonHTMLAttributes<HTMLButtonElement> & { label: string; danger?: boolean }) {
  return (
    <button
      {...rest}
      title={label}
      aria-label={label}
      className={`inline-flex h-7 w-7 items-center justify-center rounded-md transition-colors focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-accent disabled:opacity-40 ${
        danger ? "text-mute hover:bg-error/10 hover:text-error" : "text-mute hover:bg-raised hover:text-ink"
      } ${className}`}
    >
      {children}
    </button>
  );
}

export function Kbd({ children }: { children: ReactNode }) {
  return (
    <kbd className="rounded border border-line bg-raised px-1.5 py-px font-mono text-[11px] text-mute">{children}</kbd>
  );
}

export function Field({ label, hint, children }: { label: string; hint?: string; children: ReactNode }) {
  return (
    <label className="block">
      <span className="mb-1 block text-[12.5px] text-mute">{label}</span>
      {children}
      {hint && <span className="mt-1 block text-[11.5px] text-dim">{hint}</span>}
    </label>
  );
}

export const inputCls =
  "h-9 w-full rounded-md border border-line bg-raised px-3 text-[13.5px] text-ink placeholder:text-dim outline-none transition-colors focus:border-accent";

export function Input(props: React.InputHTMLAttributes<HTMLInputElement>) {
  return <input {...props} className={`${inputCls} ${props.className ?? ""}`} />;
}

export function Textarea(props: React.TextareaHTMLAttributes<HTMLTextAreaElement>) {
  return (
    <textarea
      {...props}
      className={`w-full rounded-md border border-line bg-raised px-3 py-2 font-mono text-[13px] text-ink placeholder:text-dim outline-none transition-colors focus:border-accent ${props.className ?? ""}`}
    />
  );
}

export function Segmented<T extends string>({
  value,
  options,
  onChange,
}: {
  value: T;
  options: { value: T; label: string }[];
  onChange: (v: T) => void;
}) {
  return (
    <div role="radiogroup" className="inline-flex h-9 items-stretch rounded-md border border-line bg-raised p-0.5">
      {options.map((o) => (
        <button
          key={o.value}
          role="radio"
          aria-checked={value === o.value}
          onClick={() => onChange(o.value)}
          className={`rounded px-2.5 text-[12.5px] transition-colors focus-visible:outline-2 focus-visible:outline-accent ${
            value === o.value ? "bg-surface text-ink shadow-[inset_0_0_0_1px_var(--color-line)]" : "text-mute hover:text-ink"
          }`}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

export function Toggle({ checked, onChange, label }: { checked: boolean; onChange: (v: boolean) => void; label: string }) {
  return (
    <label className="flex cursor-pointer items-center justify-between gap-4 py-1.5 text-[13.5px]">
      <span>{label}</span>
      <button
        role="switch"
        aria-checked={checked}
        onClick={(e) => {
          e.preventDefault();
          onChange(!checked);
        }}
        className={`relative h-5 w-9 shrink-0 rounded-full transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent ${
          checked ? "bg-accent" : "bg-line"
        }`}
      >
        <span
          className={`absolute top-0.5 h-4 w-4 rounded-full bg-ink transition-transform ${checked ? "translate-x-4" : "translate-x-0.5"}`}
        />
      </button>
    </label>
  );
}

export function Dialog({
  open,
  onClose,
  title,
  width = 560,
  children,
  footer,
}: {
  open: boolean;
  onClose: () => void;
  title: string;
  width?: number;
  children: ReactNode;
  footer?: ReactNode;
}) {
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);
  if (!open) return null;
  return (
    <div className="fixed inset-0 z-30 flex items-center justify-center bg-bg/70 p-6 backdrop-blur-[2px]" onMouseDown={onClose}>
      <div
        role="dialog"
        aria-modal
        aria-label={title}
        className="dialog-in flex max-h-full w-full flex-col rounded-xl border border-line bg-surface shadow-[0_24px_60px_-20px_rgba(0,0,0,.7)]"
        style={{ maxWidth: width }}
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between border-b border-line px-5 py-3">
          <h2 className="text-[15px] font-semibold">{title}</h2>
          <IconButton label="Close" onClick={onClose}>
            <X size={15} />
          </IconButton>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto px-5 py-4">{children}</div>
        {footer && <div className="flex justify-end gap-2 border-t border-line px-5 py-3">{footer}</div>}
      </div>
    </div>
  );
}
