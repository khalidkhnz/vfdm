import { loadSettings, saveSettings } from "./settings";

const dot = document.getElementById("dot")!;
const text = document.getElementById("text")!;
const enabled = document.getElementById("enabled") as HTMLInputElement;

async function main() {
  const s = await loadSettings();
  enabled.checked = s.enabled;
  enabled.addEventListener("change", async () => {
    await saveSettings({ ...(await loadSettings()), enabled: enabled.checked });
  });
  document.getElementById("options")!.addEventListener("click", () => chrome.runtime.openOptionsPage());

  const r = (await chrome.runtime.sendMessage({ type: "heartbeat" })) as { port: number | null };
  if (r?.port) {
    dot.className = "dot on";
    text.textContent = `Connected (port ${r.port})`;
  } else {
    dot.className = "dot off";
    text.textContent = "vfdm app not running";
  }
}

void main();
