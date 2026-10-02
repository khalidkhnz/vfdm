import { useEffect, useState } from "react";
import { api, type BridgeInfo } from "../lib/ipc";

export default function BridgeStatus({ onClick }: { onClick: () => void }) {
  const [info, setInfo] = useState<BridgeInfo | null>(null);

  useEffect(() => {
    let alive = true;
    const poll = () => api.getBridgeInfo().then((i) => alive && setInfo(i)).catch(() => {});
    poll();
    const t = setInterval(poll, 3000);
    return () => {
      alive = false;
      clearInterval(t);
    };
  }, []);

  const up = !!info && info.port > 0;
  return (
    <button
      onClick={onClick}
      title={up ? `Extension bridge on 127.0.0.1:${info!.port}` : "Bridge not running"}
      className="flex items-center gap-2 rounded-md px-2 py-1 text-xs text-neutral-400 hover:bg-neutral-800"
    >
      <span className={`h-2 w-2 rounded-full ${up ? "bg-emerald-400" : "bg-rose-500"}`} />
      {up ? `bridge :${info!.port}` : "bridge off"}
    </button>
  );
}
