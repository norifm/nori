// Pairing and profile sync in the browser against a real sync server (tools/sync-check.sh starts one), with a
// headless terminal (nori-cli --sync) as the other device. The page is served beside the fixture Navidrome
// (tools/dev-server.sh), so the synced profile plays in both:
//
//   python3 web/serve.py --port 8082 --server http://localhost:4533
//   node web/check/sync.mjs http://localhost:8082/ NORI_CLI SYNC_SERVER_ADDRESS [SHOTS_DIR]
//
// A new browser pairs from the welcome: it shows its code, the terminal takes it and confirms, and the
// browser opens the profile that came. Your devices lists the terminal online. Then the other way round:
// a new terminal shows its code, the browser pastes it and confirms, and the terminal gets the profile.
// The browser's Sync this profile asks before it stops, and Reset pairing leaves the group. With SHOTS_DIR,
// the sheet and Your devices are saved in Day and Night.
import { spawn } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createInterface } from "node:readline";
import puppeteer from "puppeteer-core";

const [url, cli, server, shots] = process.argv.slice(2);
if (!url || !cli || !server) {
  console.error("usage: node web/check/sync.mjs URL NORI_CLI SYNC_SERVER_ADDRESS [SHOTS_DIR]");
  process.exit(2);
}
const failures = [];
const logs = [];
const check = (ok, what) => {
  console.log(`  ${ok ? "PASS" : "FAIL"}  ${what}`);
  if (!ok) failures.push(what);
  return ok;
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const work = mkdtempSync(join(tmpdir(), "nori-web-sync."));

/** A headless terminal, one command a line, its answers read in order. */
class Terminal {
  constructor(name) {
    this.name = name;
    this.proc = spawn(cli, ["--sync", "--data", join(work, name), "--name", name], { stdio: ["pipe", "pipe", "inherit"] });
    this.answers = [];
    this.waiting = [];
    createInterface({ input: this.proc.stdout }).on("line", (line) => {
      if (line.startsWith("told ")) return;
      const w = this.waiting.shift();
      if (w) w(line);
      else this.answers.push(line);
    });
  }
  ask(command) {
    this.proc.stdin.write(command + "\n");
    return new Promise((done, failed) => {
      const t = setTimeout(() => failed(new Error(`${this.name} did not answer ${command}`)), 20000);
      this.waiting.push((line) => (clearTimeout(t), done(line)));
    });
  }
  /** Asks `command` until its answer matches `pattern`: the match, or null after `secs`. */
  async until(command, pattern, secs = 30) {
    const end = Date.now() + secs * 1000;
    let last = "";
    while (Date.now() < end) {
      last = await this.ask(command);
      const m = last.match(pattern);
      if (m) return m;
      await sleep(250);
    }
    console.log(`     (${this.name} last said: ${last})`);
    return null;
  }
  quit() {
    this.proc.stdin.end("quit\n");
  }
}

const browser = await puppeteer.launch({
  executablePath: process.env.CHROME ?? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  headless: "new",
  args: ["--mute-audio", "--disable-background-timer-throttling", "--disable-renderer-backgrounding"],
});
const terminals = [];
try {
  const ctx = await browser.createBrowserContext();
  const page = await ctx.newPage();
  await page.setViewport({ width: 1280, height: 800 });
  page.on("pageerror", (e) => logs.push(`pageerror: ${e}`));
  page.on("console", (m) => m.type() === "error" && !m.text().includes("404") && logs.push(`console: ${m.text()}`));
  await page.goto(url, { waitUntil: "load" });
  await page.waitForFunction(() => window.nori.coreMs !== undefined, { timeout: 60000 });
  await page.waitForSelector("#welcome", { timeout: 60000 });
  const ask = (call, ...args) => page.evaluate((c, a) => window.ask(c, ...a), call, args);
  const sync = async () => JSON.parse(await ask("sync_state"));
  const until = (f, arg, timeout = 30000) => page.waitForFunction(f, { timeout, polling: 250 }, arg).then(() => true, () => false);
  const shot = async (name) => shots && (await page.screenshot({ path: join(shots, `${name}.png`) }));
  await ask("sync_server", server);
  // Before any profile the page follows the system's appearance.
  await page.emulateMediaFeatures([{ name: "prefers-color-scheme", value: "light" }]);
  await shot("welcome-day");

  // A new browser pairs from the welcome with a terminal that has a profile of the page's server.
  const desk = new Terminal("Desk");
  terminals.push(desk);
  await desk.ask(`server ${server}`);
  await desk.ask(`add Home ${url.replace(/\/$/, "")} admin admin`);
  await page.click("#welcome-pair");
  check(await until(() => document.querySelector("#pair-code") && document.querySelector("#pair-words")), "the welcome shows this browser's code and its words");
  const shown = await sync();
  check(shown.pairing === "Code" && !(await page.$("#paste-link")), "a browser with no profile only shows its code");
  await shot("pair-code-day");
  await desk.ask(`take ${shown.code}`);
  const asked = await desk.until("pairing", /Confirm/);
  check(asked && shown.words.every((w) => asked.input.includes(`"${w}"`)), "the terminal is asked to confirm, with the browser's words");
  await desk.ask("confirm keep");
  check(await until(() => window.nori.page === "home", undefined, 60000), "the browser pairs and opens the profile that came");
  check((await sync()).paired, "the browser is paired");
  check(await desk.until("devices", /Browser:online/), "the terminal lists the browser online");

  // Your devices lists the terminal, in Day and in Night.
  await page.evaluate(() => (location.hash = "#/devices"));
  check(await until(() => [...document.querySelectorAll("#your-devices .group-row")].some((r) => r.textContent.includes("Desk") && r.textContent.includes("Online"))), "Your devices lists the terminal online");
  for (const [theme, name] of [["LIGHT", "day"], ["DARK", "night"]]) {
    await ask("setting", "theme", theme);
    await page.reload({ waitUntil: "load" });
    await until(() => document.querySelector("#your-devices"), undefined, 60000);
    await shot(`your-devices-${name}`);
  }

  // The other way round: a new terminal shows its code, the browser pastes it and confirms.
  const laptop = new Terminal("Laptop");
  terminals.push(laptop);
  await laptop.ask(`server ${server}`);
  const code = (await laptop.ask("code")).match(/^pairing Code (\S+) (.+)$/) ?? (await laptop.until("pairing", /^pairing Code (\S+) (.+)$/));
  check(!!code, "the new terminal shows a code");
  await page.click("#pair-device");
  await until(() => document.querySelector("#paste-link"));
  await page.click("#paste-link");
  await page.type("#pair-link", code[1]);
  await page.click("#pair-take");
  check(await until(() => document.querySelector("#pair-confirm-title")), "the browser asks before giving its sign-ins");
  const words = await page.$eval("#pair-words", (e) => e.textContent.replace(/\s+/g, " ").trim());
  check(words === code[2], "with the words the terminal shows");
  await shot("pair-confirm-night");
  await page.click("#pair-confirm");
  check(await laptop.until("pairing", /^pairing Paired/), "the terminal pairs into the browser's group");
  check(await laptop.until("profiles", /^profiles Home$/), "and has the profile");
  check(await until(() => !document.querySelector("#pair-device .body")), "the sheet closes");

  // Sync this profile off asks about the other devices; keeping it there, it stays on the terminals.
  const id = JSON.parse(await ask("profiles")).list[0].id;
  await page.evaluate((id) => (location.hash = `#/settings/Servers/${encodeURIComponent(id)}`), id);
  await until(() => document.querySelector("#sync-profile input"));
  await page.click("#sync-profile input");
  check(await until(() => document.querySelector("#confirm-other")), "Sync this profile off asks about the other devices");
  await page.click("#confirm-other");
  check(await until(() => !document.querySelector("#sync-profile input").checked), "keeping it there turns it off here");
  check(await desk.until("profiles", /^profiles Home$/), "the terminal keeps it");

  // Reset pairing leaves the group: the terminals stop seeing the browser.
  await page.evaluate(() => (location.hash = "#/devices"));
  await until(() => document.querySelector("#reset-pairing"));
  await page.click("#reset-pairing");
  await until(() => document.querySelector("#confirm-yes"));
  await page.click("#confirm-yes");
  check(await desk.until("devices", /^devices (?!.*Browser:online)/), "after Reset pairing the terminal no longer sees the browser online");
} catch (e) {
  check(false, `the check ran to its end (${e.message})`);
} finally {
  terminals.forEach((t) => t.quit());
  await browser.close();
  rmSync(work, { recursive: true, force: true });
}
for (const l of logs) console.log(`     ${l}`);
console.log(`web sync check: ${failures.length ? `${failures.length} failed` : "passed"}`);
process.exit(failures.length ? 1 : 0);
