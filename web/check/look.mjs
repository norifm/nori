// Quick screenshots of the wide layout while working on it: node web/check/look.mjs <url> <dir> [steps...]
import fs from "node:fs";
import puppeteer from "puppeteer-core";
const [url, out, ...steps] = process.argv.slice(2);
fs.mkdirSync(out, { recursive: true });
const browser = await puppeteer.launch({ executablePath: process.env.CHROME ?? "/usr/bin/chromium", headless: "new", args: ["--autoplay-policy=no-user-gesture-required", "--mute-audio"] });
const page = await browser.newPage();
page.on("console", (m) => m.type() === "error" && console.log("console:", m.text()));
await page.setViewport({ width: 1280, height: 800 });
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
const ask = (call, ...args) => page.evaluate((c, a) => window.ask(c, ...a), call, args);
const shot = (n) => page.screenshot({ path: `${out}/${n}.png` });
await page.goto(url, { waitUntil: "load" });
await page.waitForFunction(() => window.nori.coreMs !== undefined, { timeout: 30000 });
if (await page.$("#login")) await page.click("#login button");
await page.waitForFunction(() => window.nori.page === "home" && window.nori.playerReadyAt !== undefined, { timeout: 60000 });
await wait(1500);
for (const step of steps.length ? steps : ["home", "play", "bar", "full", "queue", "settings"]) {
  if (step === "home") { location: await page.evaluate(() => (location.hash = "#/home")); await wait(800); }
  else if (step === "play") {
    const albums = JSON.parse(await ask("read", JSON.stringify({ kind: "albums", sort: "alphabeticalByName", size: 500, offset: 0 }))).albums;
    const a = albums.find((a) => a.name === (process.env.ALBUM ?? "Long Album")) ?? albums[0];
    await page.evaluate((id) => (location.hash = `#/album/${id}`), a.id);
    await page.waitForSelector(".songs li .play");
    await wait(500);
    await (await page.$$(".songs li .play"))[Number(process.env.TRACK ?? 0)].click();
    await page.waitForFunction(async () => ((await window.nori.heard?.())?.seconds ?? 0) > 1, { timeout: 60000 });
    await ask("seek", 20000).catch(() => {});
    continue;
  }
  else if (step === "full") { await page.click("#bar .art"); await wait(1200); }
  else if (step.startsWith("pane:")) { await page.click(`#full [data-pane=${step.slice(5)}]`); await wait(800); }
  else if (step.startsWith("side:")) { await page.click(`#bar [data-pane=${step.slice(5)}]`); await wait(800); }
  else if (step === "close") { await page.keyboard.press("Escape"); await page.click("#full [aria-label=Close]").catch(() => {}); await wait(500); continue; }
  else if (step.startsWith("#")) { await page.evaluate((h) => (location.hash = h), step); await wait(1000); }
  else if (step.startsWith("click:")) { await page.click(step.slice(6)); await wait(600); }
  else if (step.startsWith("rclick:")) { await page.click(step.slice(7), { button: "right" }); await wait(600); }
  else if (step.startsWith("wait:")) { await wait(Number(step.slice(5))); continue; }
  await shot(step.replace(/[^a-z0-9]+/gi, "_"));
}
await browser.close();
