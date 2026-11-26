// Screenshots of the search page: the results after Enter, one kind picked, one kind only and nothing
// found; Day and Night, at 1600 x 1000, 1280 x 800 and a phone's width. Against the local fixture server:
//
//   python3 web/serve.py --port 8082 --server http://localhost:4533
//   node web/check/search-shots.mjs http://localhost:8082/ <directory> [prefix]
import fs from "node:fs";
import puppeteer from "puppeteer-core";
import { signIn } from "./signin.mjs";

const url = process.argv[2] ?? "http://localhost:8082/";
const out = process.argv[3] ?? "shots";
const prefix = process.argv[4] ?? "";
fs.mkdirSync(out, { recursive: true });
const browser = await puppeteer.launch({
  executablePath: process.env.CHROME ?? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  headless: "new",
  args: ["--mute-audio"],
});
const page = await browser.newPage();
await page.setViewport({ width: 1280, height: 800 });
const ask = (call, ...args) => page.evaluate((c, a) => window.ask(c, ...a), call, args);
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
await page.goto(url, { waitUntil: "load" });
await page.waitForFunction(() => window.nori.coreMs !== undefined, { timeout: 30000 });
await signIn(page, 60000);

/** Types `q` into the search field on show and sends it, as a person would. */
async function search(q) {
  await page.evaluate(() => (location.hash = "#/home"));
  await page.waitForFunction(() => window.nori.page === "home", { timeout: 30000 });
  let at = await page.$$eval("input[type=search]", (all) => all.findIndex((i) => i.getBoundingClientRect().width > 0));
  if (at < 0) {
    await page.evaluate(() => (location.hash = "#/search/"));
    await page.waitForFunction(() => window.nori.page === "search", { timeout: 30000 });
    at = await page.$$eval("input[type=search]", (all) => all.findIndex((i) => i.getBoundingClientRect().width > 0));
  }
  const field = (await page.$$("input[type=search]"))[at];
  await field.click({ clickCount: 3 });
  await page.keyboard.press("Backspace");
  await page.keyboard.type(q, { delay: 40 });
  await page.keyboard.press("Enter");
  await page.waitForFunction(() => window.nori.page === "search", { timeout: 30000 });
  await wait(2500);
  await page.mouse.move(1, 1);
  await page.evaluate(() => document.activeElement?.blur());
  await wait(300);
}

for (const theme of ["LIGHT", "DARK"]) {
  await ask("setting", "theme", theme);
  const look = theme === "LIGHT" ? "day" : "night";
  for (const [w, h, scale, size] of [[1600, 1000, 1, "1600x1000"], [1280, 800, 1, "1280x800"], [390, 844, 2, "phone"]]) {
    await page.setViewport({ width: w, height: h, deviceScaleFactor: scale, isMobile: size === "phone", hasTouch: size === "phone" });
    await page.reload({ waitUntil: "load" });
    await page.waitForFunction(() => window.nori.page !== undefined, { timeout: 60000 });
    for (const q of ["alpha", "first", "track", "zzzz"]) {
      await search(q);
      await page.screenshot({ path: `${out}/${prefix}web-${size}-${look}-${q}.png` });
      // A kind picked: Albums, where the page has kind chips.
      const chip = await page.$("#search-kinds [data-kind=albums]");
      if (chip && q === "first") {
        await chip.click();
        await wait(800);
        await page.screenshot({ path: `${out}/${prefix}web-${size}-${look}-${q}-albums.png` });
      }
    }
  }
}
await ask("setting", "theme", "SYSTEM");
await browser.close();
