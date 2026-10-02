import { findPort, sendDownload } from "./bridge";
import { loadSettings, saveSettings, type ExtSettings, type OfflinePolicy } from "./settings";

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

function readForm(): ExtSettings {
  return {
    enabled: $<HTMLInputElement>("enabled").checked,
    token: $<HTMLInputElement>("token").value.trim(),
    port: Number($<HTMLInputElement>("port").value) || 0,
    extensions: $<HTMLTextAreaElement>("extensions")
      .value.split(/[,\s]+/)
      .map((s) => s.trim().replace(/^\./, "").toLowerCase())
      .filter(Boolean),
    minSizeMb: Math.max(0, Number($<HTMLInputElement>("minSize").value) || 0),
    skipHosts: $<HTMLTextAreaElement>("skipHosts")
      .value.split(/\r?\n/)
      .map((s) => s.trim().toLowerCase())
      .filter(Boolean),
    offlinePolicy: $<HTMLSelectElement>("offline").value as OfflinePolicy,
    notify: $<HTMLInputElement>("notify").checked,
  };
}

function fillForm(s: ExtSettings) {
  $<HTMLInputElement>("enabled").checked = s.enabled;
  $<HTMLInputElement>("token").value = s.token;
  $<HTMLInputElement>("port").value = String(s.port);
  $<HTMLTextAreaElement>("extensions").value = s.extensions.join(", ");
  $<HTMLInputElement>("minSize").value = String(s.minSizeMb);
  $<HTMLTextAreaElement>("skipHosts").value = s.skipHosts.join("\n");
  $<HTMLSelectElement>("offline").value = s.offlinePolicy;
  $<HTMLInputElement>("notify").checked = s.notify;
}

async function test() {
  const status = $("status");
  status.className = "";
  status.textContent = "…";
  const s = readForm();
  const port = await findPort(s.port);
  if (port === null) {
    status.className = "bad";
    status.textContent = "App not reachable. Is vfdm running?";
    return;
  }
  const r = await sendDownload(port, s.token, { url: "https://example.invalid/dry" }, true);
  if (r.ok) {
    status.className = "ok";
    status.textContent = `Connected on port ${port}`;
  } else if (r.reason === "unauthorized") {
    status.className = "bad";
    status.textContent = `Found app on port ${port}, but the token is wrong`;
  } else {
    status.className = "bad";
    status.textContent = `Error: ${r.detail ?? r.reason}`;
  }
}

async function main() {
  fillForm(await loadSettings());
  $("test").addEventListener("click", () => void test());
  $("save").addEventListener("click", async () => {
    await saveSettings(readForm());
    const el = $("saved");
    el.textContent = "Saved";
    setTimeout(() => (el.textContent = ""), 1500);
  });
}

void main();
