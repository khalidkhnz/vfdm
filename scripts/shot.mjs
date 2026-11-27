// Screenshot the desktop UI running in the browser mock (pnpm --filter @vfdm/desktop dev:web).
// Usage: node scripts/shot.mjs <out.png> [url] [width] [height] [waitMs] [--click="selector"]
import puppeteer from "puppeteer-core";

const [out = "ui.png", url = "http://localhost:1420/", w = "1100", h = "700", wait = "2500", ...rest] = process.argv.slice(2);
const clicks = rest.filter((a) => a.startsWith("--click=")).map((a) => a.slice(8));
const emulate = rest.find((a) => a.startsWith("--reduced-motion")) ? "reduce" : "no-preference";
const chrome = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";

const browser = await puppeteer.launch({ executablePath: chrome, headless: true, args: ["--hide-scrollbars"] });
const page = await browser.newPage();
await page.setViewport({ width: +w, height: +h, deviceScaleFactor: 2 });
await page.emulateMediaFeatures([{ name: "prefers-reduced-motion", value: emulate }]);
const errors = [];
page.on("console", (m) => { if (m.type() === "error" || m.type() === "warning") errors.push(`${m.type()}: ${m.text()}`); });
page.on("pageerror", (e) => errors.push(`pageerror: ${e.message}`));
page.on("response", (r) => { if (r.status() >= 400) errors.push(`${r.status()}: ${r.url()}`); });
await page.goto(url, { waitUntil: "networkidle0", timeout: 30000 });
for (const sel of clicks) { await page.click(sel); await new Promise((r) => setTimeout(r, 300)); }
await new Promise((r) => setTimeout(r, +wait));
await page.screenshot({ path: out });
await browser.close();
console.log(`wrote ${out}`);
if (errors.length) { console.log("console:"); for (const e of errors) console.log("  " + e); }
