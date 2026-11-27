import { useEffect, useState } from "react";
import { Check, Copy, RefreshCw } from "lucide-react";
import { api, events, type AppSettings, type BridgeInfo, type ToolName, type ToolStatus } from "../lib/ipc";
import { Button, Dialog, Field, Input, Toggle } from "../components/ui";

interface Props {
  open: boolean;
  onClose: () => void;
  onSaved: (s: AppSettings) => void;
}

const TOOL: Record<ToolName, { title: string; why: string }> = {
  ffmpeg: { title: "ffmpeg", why: "Turns HLS .ts into .mp4 and joins DASH video with audio." },
  ytdlp: { title: "yt-dlp", why: "Downloads from YouTube, Vimeo and other sites that need site-specific logic." },
  node: { title: "Node.js", why: "yt-dlp needs a JavaScript runtime for YouTube." },
};

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="border-b border-line py-4 first:pt-0 last:border-b-0 last:pb-0">
      <h3 className="mb-3 text-[13px] font-semibold">{title}</h3>
      {children}
    </section>
  );
}

function ToolRow({
  t,
  busy,
  onInstall,
  onBrowse,
  onClear,
}: {
  t: ToolStatus;
  busy: boolean;
  onInstall: () => void;
  onBrowse: () => void;
  onClear: () => void;
}) {
  const label = TOOL[t.name];
  const found = !!t.path;
  return (
    <div className="flex items-start gap-3 py-2">
      <span className={`mt-1.5 h-1.5 w-1.5 shrink-0 rounded-full ${found ? "bg-ok" : "bg-error"}`} />
      <div className="min-w-0 flex-1">
        <div className="flex items-baseline gap-2">
          <span className="text-[13.5px] font-medium">{label.title}</span>
          <span className="font-mono text-[11.5px] text-dim">
            {found ? `${t.version ?? ""} ${t.source === "path" ? "on this Mac" : t.source === "bundled" ? "installed by vfdm" : "custom path"}`.trim() : "not found"}
          </span>
        </div>
        <div className="text-[12px] text-mute">{label.why}</div>
        {t.path && (
          <div className="mt-0.5 truncate font-mono text-[11px] text-dim" title={t.path}>
            {t.path}
          </div>
        )}
        {t.hint && <div className="mt-1 text-[12px] text-warn">{t.hint}</div>}
      </div>
      <div className="flex shrink-0 gap-1">
        {t.installable && (
          <Button onClick={onInstall} disabled={busy} className="h-7 px-2.5 text-[12px]">
            {busy ? "Installing…" : found && t.source === "bundled" ? "Update" : "Install"}
          </Button>
        )}
        {t.name !== "node" && (
          <Button variant="ghost" onClick={onBrowse} className="h-7 px-2.5 text-[12px]">
            Choose file…
          </Button>
        )}
        {t.source === "override" && (
          <Button variant="ghost" onClick={onClear} className="h-7 px-2 text-[12px]" title="Back to auto-detect">
            Reset
          </Button>
        )}
      </div>
    </div>
  );
}

export default function Settings({ open, onClose, onSaved }: Props) {
  const [s, setS] = useState<AppSettings | null>(null);
  const [bridge, setBridge] = useState<BridgeInfo | null>(null);
  const [tools, setTools] = useState<ToolStatus[]>([]);
  const [installing, setInstalling] = useState<ToolName | null>(null);
  const [copied, setCopied] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    api.getSettings().then(setS);
    api.getBridgeInfo().then(setBridge);
    api.getTools().then(setTools);
    setError(null);
    const un = events.onToolsChanged(setTools);
    return () => {
      void un.then((f) => f());
    };
  }, [open]);

  if (!s) return null;

  const save = async () => {
    try {
      const saved = await api.setSettings(s);
      onSaved(saved);
      onClose();
    } catch (e) {
      setError(String(e));
    }
  };

  const copyToken = async () => {
    if (!bridge) return;
    await navigator.clipboard.writeText(bridge.token);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };

  const install = async (name: ToolName) => {
    setInstalling(name);
    setError(null);
    try {
      await api.installTool(name);
      setTools(await api.getTools());
    } catch (e) {
      setError(`Install failed: ${String(e)}`);
    } finally {
      setInstalling(null);
    }
  };

  const setToolPath = async (name: ToolName, p: string | null) => {
    const next = name === "ffmpeg" ? { ...s, ffmpeg_path: p } : { ...s, ytdlp_path: p };
    setS(next);
    try {
      const saved = await api.setSettings(next);
      onSaved(saved);
      setTools(await api.getTools());
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <Dialog
      open={open}
      onClose={onClose}
      title="Settings"
      width={620}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" onClick={save}>
            Save changes
          </Button>
        </>
      }
    >
      <Section title="Downloads">
        <div className="flex flex-col gap-3">
          <Field label="Save files to">
            <div className="flex gap-2">
              <Input value={s.download_dir} onChange={(e) => setS({ ...s, download_dir: e.target.value })} />
              <Button
                onClick={async () => {
                  const p = await api.pickDownloadDir();
                  if (p) setS({ ...s, download_dir: p });
                }}
                className="h-9 shrink-0"
              >
                Choose…
              </Button>
            </div>
          </Field>
          <div className="grid grid-cols-2 gap-3">
            <Field label="Connections per download" hint="1–16. More helps on throttled servers.">
              <Input
                type="number"
                min={1}
                max={16}
                value={s.max_connections}
                onChange={(e) => setS({ ...s, max_connections: Number(e.target.value) || 1 })}
              />
            </Field>
            <Field label="Downloads at once" hint="1–10. The rest wait in line.">
              <Input
                type="number"
                min={1}
                max={10}
                value={s.max_concurrent}
                onChange={(e) => setS({ ...s, max_concurrent: Number(e.target.value) || 1 })}
              />
            </Field>
          </div>
        </div>
      </Section>

      <Section title="Behaviour">
        <Toggle checked={s.notify_on_complete} onChange={(v) => setS({ ...s, notify_on_complete: v })} label="Notify when a download finishes" />
        <Toggle checked={s.focus_on_capture} onChange={(v) => setS({ ...s, focus_on_capture: v })} label="Bring vfdm to the front when the browser sends a download" />
      </Section>

      <Section title="Tools">
        <p className="mb-1 text-[12.5px] text-mute">Found automatically on this computer. Install puts a private copy in vfdm's data folder.</p>
        <div className="divide-y divide-line">
          {tools.map((t) => (
            <ToolRow
              key={t.name}
              t={t}
              busy={installing === t.name}
              onInstall={() => install(t.name)}
              onBrowse={async () => {
                const p = await api.pickToolPath();
                if (p) await setToolPath(t.name, p);
              }}
              onClear={() => setToolPath(t.name, null)}
            />
          ))}
        </div>
      </Section>

      <Section title="Browser extension">
        <p className="text-[12.5px] text-mute">
          Paste this token into the vfdm extension's options page. The app listens on{" "}
          <span className="font-mono text-ink">127.0.0.1:{bridge?.port || "…"}</span>.
        </p>
        <div className="mt-2 flex items-center gap-2">
          <code className="min-w-0 flex-1 truncate rounded-md border border-line bg-raised px-2.5 py-1.5 font-mono text-[12px] text-mute">
            {bridge?.token ?? "…"}
          </code>
          <Button onClick={copyToken} className="h-8 shrink-0">
            {copied ? <Check size={14} className="text-ok" /> : <Copy size={14} />}
            {copied ? "Copied" : "Copy"}
          </Button>
          <Button
            variant="ghost"
            onClick={async () => setBridge(await api.regenerateToken())}
            className="h-8 shrink-0"
            title="The old token stops working immediately"
          >
            <RefreshCw size={14} />
            New token
          </Button>
        </div>
      </Section>

      {error && <p className="mt-3 text-[12.5px] text-error">{error}</p>}
    </Dialog>
  );
}
