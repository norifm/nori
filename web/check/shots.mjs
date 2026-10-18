// Screenshots of the web client at a desktop's and a phone's width, light and dark: Home, Library, an
// album, an artist, search, the player bar, the full player with lyrics and with the queue, Settings
// and a jam. Against a server with the relay (for the jam):
//
//   python3 web/serve.py --port 8081 --server http://localhost:5274
//   node web/check/shots.mjs http://localhost:8081/ <directory>
import fs from "node:fs";
import puppeteer from "puppeteer-core";

const url = process.argv[2] ?? "http://localhost:8081/";
const out = process.argv[3] ?? "shots";
fs.mkdirSync(out, { recursive: true });
const browser = await puppeteer.launch({
  executablePath: process.env.CHROME ?? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  headless: "new",
  args: ["--autoplay-policy=no-user-gesture-required", "--mute-audio"],
});
const page = await browser.newPage();
await page.setViewport({ width: 1280, height: 800 });
const ask = (call, ...args) => page.evaluate((c, a) => window.ask(c, ...a), call, args);
const read = async (q) => JSON.parse(await ask("read", JSON.stringify(q)));
const wait = (ms) => new Promise((r) => setTimeout(r, ms));

/** Opens `h` and waits for its page (`top`) to be drawn and its covers to load. */
async function go(h, top) {
  await page.evaluate((x) => {
    window.nori.page = undefined;
    location.hash = x;
  }, h);
  await page.waitForFunction((t) => window.nori.page === t, { timeout: 30000 }, top);
  await page.waitForFunction(() => [...document.images].every((i) => {
    const r = i.getBoundingClientRect();
    return i.complete || r.bottom <= 0 || r.right <= 0 || r.top >= innerHeight || r.left >= innerWidth;
  }), { timeout: 10000 }).catch(() => {});
  await wait(250);
}

await page.goto(url, { waitUntil: "load" });
await page.waitForFunction(() => window.nori.coreMs !== undefined, { timeout: 30000 });
if (await page.$("#login")) await page.click("#login button");
await page.waitForFunction(() => window.nori.page === "home" && window.nori.playerReadyAt !== undefined, { timeout: 60000 });
const albums = (await read({ kind: "albums", sort: "alphabeticalByName", size: 500, offset: 0 })).albums;
const sung = albums.find((a) => a.name === "Sung Album") ?? albums[0];
const scrollAlbum = albums.find((a) => a.name === "Second Wind" && a.artist === "Alpha Waves") ?? sung;
const artist = sung.artistId ?? (await read({ kind: "artists" })).artists[0].id;

// Something plays, with lyrics, and a jam is hosted.
await go(`#/album/${sung.id}`, "album");
await (await page.$$(".songs li .play"))[0].click();
await page.waitForFunction(() => (window.nori.heard?.seconds ?? 0) > 1, { timeout: 60000 });
await page.click("#bar [data-pane=lyrics]");
await page.waitForFunction(() => window.nori.lyrics, { timeout: 30000 });
await page.click("#panel [aria-label=Close]");
await ask("jam_open").catch(() => {});
await wait(1500);

const shot = (name) => page.screenshot({ path: `${out}/${name}.png` });
for (const scheme of ["light", "dark"]) {
  await page.emulateMediaFeatures([{ name: "prefers-color-scheme", value: scheme }]);
  for (const [width, height, scale, size] of [[1280, 800, 1, "desktop"], [390, 844, 2, "phone"]]) {
    await page.setViewport({ width, height, deviceScaleFactor: scale, isMobile: size === "phone", hasTouch: size === "phone" });
    const name = (what) => `${what}-${size}-${scheme}`;
    await go("#/home", "home");
    await shot(name("home"));
    await go("#/library", "library");
    await shot(name("library"));
    await go(`#/album/${sung.id}`, "album");
    await shot(name("album"));
    if (size === "phone") {
      if (scrollAlbum.id !== sung.id) await go(`#/album/${scrollAlbum.id}`, "album");
      const reachable = await page.$$eval(".songs li .play", (rows) => {
        const last = rows.at(-1);
        last.scrollIntoView({ block: "center" });
        const r = last.getBoundingClientRect();
        return last.contains(document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2));
      });
      if (!reachable) throw new Error("the phone's last song is covered by the mini-player");
      await shot(name("album-bottom"));
    }
    await go(`#/artist/${artist}`, "artist");
    await shot(name("artist"));
    await go("#/search/light", "search");
    await page.waitForFunction(() => window.nori.searched?.query === "light", { timeout: 30000 });
    await wait(300);
    await shot(name("search"));
    await go("#/settings", "settings");
    await shot(name("settings"));
    if (size === "desktop") {
      // The player bar with the lyrics beside the page, then the queue, then the jam.
      await go(`#/album/${sung.id}`, "album");
      await page.click("#bar [data-pane=lyrics]");
      await wait(800);
      await shot(name("player-bar-lyrics"));
      await page.click("#bar [data-pane=queue]");
      await wait(400);
      await shot(name("player-bar-queue"));
      await page.click("#bar [data-pane=jam]");
      await wait(400);
      await shot(name("jam"));
      await page.click("#panel [aria-label=Close]");
      await page.click("#bar .art");
    } else {
      await shot(name("mini-player"));
      await page.click("#mini");
    }
    // The full player: as it opens, with the lyrics, with the queue.
    await page.waitForSelector("#full");
    await wait(600);
    if (size === "phone") {
      await shot(name("now-playing"));
      await page.click("#full [data-pane=lyrics]");
      await wait(800);
    }
    await shot(name("now-playing-lyrics"));
    await page.click("#full [data-pane=queue]");
    await wait(400);
    await shot(name("now-playing-queue"));
    if (size === "phone") {
      await page.click("#full [data-pane=jam]");
      await wait(400);
      await shot(name("jam"));
    }
    await page.click("#full [aria-label=Close]");
  }
}
await ask("jam_close").catch(() => {});
console.log(fs.readdirSync(out).filter((f) => f.endsWith(".png")).length, "screenshots in", out);
await browser.close();
