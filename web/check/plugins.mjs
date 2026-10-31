// The plugin host in headless Chrome, against web/serve.py and the local Navidrome: nothing of it loads with
// no plugin; blocked plugins are greyed with their reasons; orbit installs from the repo, shows its rows,
// changes the samples the AudioWorklet plays, survives a reload and goes on Remove; playback sets the
// engine's speed through an override; moving-covers reaches
// Apple through the server and its loop plays in a <video>.
//
//   node web/check/plugins.mjs [http://localhost:8099/]
//   NORI_INDEX=index.json node web/check/plugins.mjs ...   (the repo's index read from this file)
import fs from "node:fs";
import puppeteer from "puppeteer-core";

const url = process.argv[2] ?? "http://localhost:8099/";
const browser = await puppeteer.launch({ executablePath: process.env.CHROME ?? "/usr/bin/chromium", headless: "new", args: ["--no-sandbox", "--autoplay-policy=no-user-gesture-required", "--mute-audio"] });
const page = await browser.newPage();
await page.setViewport({ width: 1280, height: 800 });
const fetched = [];
const index = process.env.NORI_INDEX && fs.readFileSync(process.env.NORI_INDEX, "utf8");
/** The page's server has no plugin route: a static host. */
let noProxy = false;
await page.setRequestInterception(true);
page.on("request", (r) => {
  fetched.push(r.url());
  if (index && r.url().endsWith("nori-plugins/main/index.json")) return r.respond({ status: 200, contentType: "application/json", headers: { "Access-Control-Allow-Origin": "*" }, body: index });
  if (noProxy && r.url().includes("/plugin-http")) return r.respond({ status: 404, body: "" });
  r.continue();
});
page.on("pageerror", (e) => console.log(`  pageerror: ${e}`));
page.on("console", (m) => {
  if (m.type() === "error" || m.text().startsWith("[fm.nori") || m.text().includes("plugin")) console.log(`  ${m.type()}: ${m.text().slice(0, 300)}`);
});

let failures = 0;
function check(what, ok, seen = "") {
  console.log(`${ok ? "ok  " : "FAIL"} ${what}${seen === "" ? "" : `: ${typeof seen === "string" ? seen : JSON.stringify(seen)}`}`);
  if (!ok) failures++;
}
const ask = (call, ...args) => page.evaluate((c, a) => window.ask(c, ...a), call, args);
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
const heard = () => ask("heard");
const hostLoaded = () => fetched.filter((u) => /js-component-bindgen|\/(install|runner|hls)-/.test(u)).map((u) => u.split("/").pop());
const hash = (h) => page.evaluate((x) => (location.hash = x), h);
const clickText = (selector) => page.$eval(selector, (e) => e.click());

async function start() {
  await page.goto(url, { waitUntil: "load" });
  await page.waitForFunction(() => document.querySelector("#login") || window.nori?.page === "home", { timeout: 30000 });
  await page.waitForFunction(() => window.nori.page === "home" || document.querySelector("#login"), { timeout: 30000 });
  if (await page.$("#login")) await page.click("#login button");
  await page.waitForFunction(() => window.nori.page === "home", { timeout: 30000 });
}

async function play() {
  const albums = JSON.parse(await ask("read", JSON.stringify({ kind: "albums", sort: "alphabeticalByName", size: 200, offset: 0 })));
  const list = albums.albums ?? albums;
  await hash(`#/album/${(list.find((a) => a.name === "Long Album") ?? list[0]).id}`);
  await page.waitForSelector(".songs li .play");
  await wait(400);
  await (await page.$$(".songs li .play"))[0].click();
  await page.waitForFunction(async () => ((await window.nori.heard?.())?.seconds ?? 0) > 1, { timeout: 60000 });
  // The library's quiet tracks say nothing of a stage.
  for (let tries = 0; tries < 6 && (await heard()).peak < 0.01; tries++) {
    console.log('  silent:', await page.evaluate(() => window.nori.page + ' ' + JSON.stringify(document.querySelector('#bar')?.innerText?.slice(0, 60))));
    await ask("next");
    await wait(4000);
  }
}

const plugins = "#/settings/Plugins";
/** Settings › Plugins, fresh, its list showing. */
const listed = async () => {
  await hash("#/home");
  await wait(300);
  await hash(plugins);
};
await start();
await wait(800);
check("no plugin: nothing of the host is fetched", hostLoaded().length === 0, hostLoaded());
check("no plugin: the host's generator isn't precached", !(await page.evaluate(async () => (await (await fetch("sw.js")).text()).match(/js-component-bindgen|runner-/))));

await hash(plugins);
await page.waitForSelector("[data-plugin]", { timeout: 20000 });
for (const [id, wants] of [...(index ? [["fm.nori.speaker", "Blocked"]] : []), ["fm.nori.youtube-music", "Blocked"]]) {
  const row = await page.$eval(`[data-plugin="${id}"]`, (e) => ({ fit: e.dataset.fit, greyed: e.classList.contains("off"), why: [...e.querySelectorAll(".why")].map((w) => w.textContent) }));
  check(`${id} is greyed with its reasons`, row.fit === wants && row.greyed && row.why.length > 0, row.why);
}

async function install(id) {
  await page.waitForSelector(`[data-plugin="${id}"] [data-install]`, { timeout: 20000 });
  await clickText(`[data-plugin="${id}"] [data-install]`);
  await page.waitForSelector(`[data-installed="${id}"]`, { timeout: 60000 });
}
await install("fm.nori.orbit");
check("the generator came with the install", hostLoaded().some((f) => f.includes("js-component-bindgen")), hostLoaded());

const open = async (id) => {
  await listed();
  await page.waitForSelector(`[data-installed="${id}"]`, { timeout: 20000 });
  await clickText(`[data-installed="${id}"]`);
  await page.waitForSelector(`[data-plugin-page="${id}"]`);
};
await open("fm.nori.orbit");
const rows = await page.$$eval("[data-plugin-page] [data-row]", (r) => r.map((e) => e.dataset.row));
check("orbit's page shows its rows", rows.length > 0, rows);
await clickText("[data-plugin-on]");
await wait(300);

await play();
await wait(6000);
const on = await heard();
check("orbit changes the samples the worklet plays", on.staged > 10000 && on.moved > 1 && on.stagedPeak[0] !== on.stagedPeak[1], { staged: on.staged, moved: on.moved, peak: on.stagedPeak });

await page.reload({ waitUntil: "load" });
await start();
await wait(800);
check("a reload starts it again", hostLoaded().length > 0 && (await page.evaluate(() => !!window.plugins)), "");
await play();
await wait(6000);
const back = await heard();
check("orbit plays after the reload", back.staged > 10000 && back.moved > 1, { staged: back.staged, moved: back.moved, peak: back.stagedPeak });

await open("fm.nori.orbit");
await clickText("[data-plugin-on]");
await wait(500);
const [a, b] = [await heard(), (await wait(1500), await heard())];
check("off, it stops touching the sound", a.staged === b.staged, [a.staged, b.staged]);

await listed();
await install("fm.nori.playback");
await open("fm.nori.playback");
await clickText("[data-plugin-on]");
await wait(800);
/** How much of the song passes in 3 s of the clock, and the pace the session says. */
const paced = async () => {
  const now = async () => JSON.parse(await ask("now"));
  const a = await now();
  await wait(3000);
  const b = await now();
  return { pace: b.pace, passed: (b.positionMs - a.positionMs) / 3000 };
};
const setSpeed = (v) => page.evaluate((v) => { const p = window.plugins; return p.setRow("fm.nori.playback", p.find("fm.nori.playback").plugin.rows.find((r) => r.id === "speed"), v); }, v);
await setSpeed(1.5);
await wait(1500);
const fast = await paced();
check("speed reaches the engine", Math.abs(fast.pace - 1.5) < 0.01 && Math.abs(fast.passed - 1.5) < 0.2, fast);
await page.reload({ waitUntil: "load" });
await start();
await wait(800);
await play();
await wait(1500);
const after = await paced();
check("speed stays after a reload", Math.abs(after.pace - 1.5) < 0.01, after);
await open("fm.nori.playback");
await clickText("[data-plugin-on]");
await wait(1500);
const plain = await paced();
check("off, nori's own speed is back", Math.abs(plain.pace - 1) < 0.01 && Math.abs(plain.passed - 1) < 0.2, plain);

await listed();
await install("fm.nori.moving-covers");
await open("fm.nori.moving-covers");
await clickText("[data-plugin-on]");
await wait(1500);
const sleeve = await page.evaluate(async (albums) => {
  const p = window.plugins;
  const now = window.player.song;
  for (const [artist, album] of albums) {
    const s = await p.sleeve({ ...now, albumId: album, artist, album, title: album });
    if (s?.video) return { album, video: s.video };
  }
  return null;
}, [["Billie Eilish", "HIT ME HARD AND SOFT"], ["Taylor Swift", "Midnights"], ["Bad Bunny", "Un Verano Sin Ti"], ["The Weeknd", "Dawn FM"]]);
check("moving covers finds a loop on Apple through the server", !!sleeve, sleeve);
if (sleeve) {
  await page.click("#bar .art");
  await page.waitForSelector("video[data-sleeve]", { timeout: 10000 });
  await page.waitForFunction(() => { const v = document.querySelector("video[data-sleeve]"); return v && v.currentTime > 0.3 && v.videoWidth > 0; }, { timeout: 30000 }).catch(() => {});
  const v = await page.$eval("video[data-sleeve]", (e) => ({ t: e.currentTime, w: e.videoWidth, muted: e.muted, loop: e.loop, error: e.error?.message }));
  check("its loop plays over the cover", v.t > 0.3 && v.w > 0, v);
  await page.keyboard.press("Escape");
}

for (const id of ["fm.nori.moving-covers", "fm.nori.playback", "fm.nori.orbit"]) {
  await open(id);
  await clickText("[data-plugin-remove]");
  await listed();
  await page.waitForFunction((x) => !document.querySelector(`[data-installed="${x}"]`), {}, id);
}
fetched.length = 0;
await page.reload({ waitUntil: "load" });
await start();
await wait(800);
check("removed: nothing of the host loads", hostLoaded().length === 0 && !(await page.evaluate(() => localStorage.getItem("nori-plugins"))), hostLoaded());

if (index) {
  noProxy = true;
  await page.reload({ waitUntil: "load" });
  await start();
  await hash(plugins);
  await page.waitForSelector("[data-plugin]", { timeout: 20000 });
  const covers = await page.$eval('[data-plugin="fm.nori.moving-covers"]', (e) => ({ fit: e.dataset.fit, why: e.querySelector(".why")?.textContent, install: !!e.querySelector("[data-install]") }));
  check("no route on the server: a plugin that names hosts is greyed", covers.fit === "Blocked" && !covers.install, covers);
}

await browser.close();
console.log(failures ? `${failures} failed` : "all good");
process.exit(failures ? 1 : 0);
