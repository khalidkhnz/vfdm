import { useEffect, useState } from "react";
import { api, type Kind } from "../lib/ipc";
import { Button, Dialog, Field, Input, Segmented, Textarea } from "./ui";

interface Props {
  open: boolean;
  onClose: () => void;
  defaultDir: string;
  defaultConnections: number;
  initialUrls?: string;
}

const KINDS: { value: Kind; label: string }[] = [
  { value: "auto", label: "Auto" },
  { value: "file", label: "File" },
  { value: "hls", label: "HLS" },
  { value: "dash", label: "DASH" },
  { value: "ytdlp", label: "yt-dlp" },
];

export default function AddUrlDialog({ open, onClose, defaultDir, defaultConnections, initialUrls }: Props) {
  const [urls, setUrls] = useState("");
  const [filename, setFilename] = useState("");
  const [dir, setDir] = useState(defaultDir);
  const [conns, setConns] = useState(defaultConnections);
  const [kind, setKind] = useState<Kind>("auto");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setDir(defaultDir);
    setConns(defaultConnections);
    setKind("auto");
    setError(null);
    if (initialUrls) {
      setUrls(initialUrls);
      return;
    }
    navigator.clipboard
      ?.readText()
      .then((t) => {
        if (/^https?:\/\/\S+$/i.test(t.trim())) setUrls(t.trim());
      })
      .catch(() => {});
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  const list = urls
    .split(/\r?\n/)
    .map((s) => s.trim())
    .filter(Boolean);

  const submit = async () => {
    if (list.length === 0) return setError("Paste at least one link.");
    const bad = list.find((u) => !/^https?:\/\//i.test(u));
    if (bad) return setError(`Not a web link: ${bad}`);
    try {
      for (const url of list) {
        await api.add({
          url,
          filename: list.length === 1 && filename ? filename : undefined,
          dest_dir: dir || undefined,
          max_connections: conns,
          kind,
        });
      }
      setUrls("");
      setFilename("");
      onClose();
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <Dialog
      open={open}
      onClose={onClose}
      title="Add download"
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" onClick={submit}>
            {list.length > 1 ? `Download ${list.length} files` : "Download"}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-4">
        <Field label="Links, one per line">
          <Textarea
            autoFocus
            rows={4}
            spellCheck={false}
            value={urls}
            onChange={(e) => setUrls(e.target.value)}
            onKeyDown={(e) => {
              if ((e.metaKey || e.ctrlKey) && e.key === "Enter") void submit();
            }}
            placeholder="https://example.com/file.zip"
          />
        </Field>
        <div className="flex flex-wrap items-end gap-4">
          <Field label="Treat as" hint="Auto reads the link and the server's response.">
            <Segmented value={kind} options={KINDS} onChange={setKind} />
          </Field>
          <div className="w-28">
            <Field label="Connections">
              <Input
                type="number"
                min={1}
                max={16}
                value={conns}
                onChange={(e) => setConns(Math.min(16, Math.max(1, Number(e.target.value) || 1)))}
              />
            </Field>
          </div>
        </div>
        {list.length <= 1 && (
          <Field label="Save as" hint="Leave empty to use the server's name.">
            <Input value={filename} onChange={(e) => setFilename(e.target.value)} placeholder="optional" />
          </Field>
        )}
        <Field label="Folder">
          <div className="flex gap-2">
            <Input value={dir} onChange={(e) => setDir(e.target.value)} />
            <Button
              onClick={async () => {
                const picked = await api.pickDownloadDir();
                if (picked) setDir(picked);
              }}
              className="h-9 shrink-0"
            >
              Choose…
            </Button>
          </div>
        </Field>
        {error && <p className="text-[12.5px] text-error">{error}</p>}
      </div>
    </Dialog>
  );
}
