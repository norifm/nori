// The web client in headless Chrome against the local Navidrome (tools/dev-server.sh) through
// web/serve.py: sign-in, browsing, search, a song streamed and played, next and previous, synced lyrics,
// a crossfade and an AutoMix transition heard as continuous samples, speed and pitch. Exits non-zero
// when something is off, and prints what it saw.
//
//   npm --prefix web/check install && node web/check/check.mjs [http://localhost:8080/] [phase]
//   SLOW_URL=http://localhost:8081/ node web/check/check.mjs ...   (also a Play during a slow load)
//   THROTTLED_URL=http://localhost:8082/ node web/check/check.mjs ...   (also a song started at 400 kbit/s)
//
// CHROME names the browser (default: Google Chrome's macOS path). Headless Chrome is muted: what the
// AudioWorklet played is checked instead (frames, running dry, silences over 20 ms).
import fs from "node:fs";
import puppeteer from "puppeteer-core";

const url = process.argv[2] ?? "http://localhost:8080/";
// Phases by name, comma separated; all without.
const only = process.argv[3]?.split(",");
// A second address of the client through a throttled server (`web/serve.py --port 8081 --kbps 4000
// --files-kbps 4000`).
const slow = process.env.SLOW_URL;
const browser = await puppeteer.launch({
  executablePath: process.env.CHROME ?? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  headless: "new",
  args: ["--autoplay-policy=no-user-gesture-required", "--mute-audio"],
});
const page = await browser.newPage();
await page.setViewport({ width: 1280, height: 800 });
const logs = [];
page.on("pageerror", (e) => logs.push(`pageerror: ${e}`));
page.on("console", (m) => m.type() === "error" && !m.text().includes("404") && logs.push(`console: ${m.text()}`));
page.on("workercreated", (w) => w.on("console", (m) => m.type() !== "log" && logs.push(`worker ${m.type()}: ${m.text()}`)));
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
const facts = () => page.evaluate(() => JSON.parse(JSON.stringify(window.nori)));
const ask = (call, ...args) => page.evaluate((c, a) => window.ask(c, ...a), call, args);
const now = async () => JSON.parse(await ask("now"));
const read = async (q) => JSON.parse(await ask("read", JSON.stringify(q)));
const report = {};
const failures = [];
const expect = (ok, what) => ok || failures.push(what);

/** Opens `h` and waits for its page (`top`) to be drawn. */
async function hash(h, top) {
  await page.evaluate((x) => {
    window.nori.page = undefined;
    // The same address again draws the page again.
    if (location.hash === x) window.dispatchEvent(new HashChangeEvent("hashchange"));
    else location.hash = x;
  }, h);
  await page.waitForFunction((t) => window.nori.page === t, { timeout: 30000 }, top);
}

await page.goto(url, { waitUntil: "load" });
await page.waitForFunction(() => window.nori.coreMs !== undefined, { timeout: 30000 });
if (await page.$("#login")) await page.click("#login button");
await page.waitForFunction(() => window.nori.page === "home", { timeout: 30000 });
const start = await facts();
report.start = { coreMs: start.coreMs, loginMs: start.loginMs, homeAlbums: start.albums };
expect(start.albums > 0, "the home page shows albums");

const albums = (await read({ kind: "albums", sort: "alphabeticalByName", size: 500, offset: 0 })).albums;
const album = (name, artist) => albums.find((a) => a.name === name && (!artist || a.artist === artist))?.id;

if (!only || only.includes("browse")) {
  const counts = {};
  const firstArtist = (await read({ kind: "artists" })).artists[0]?.id;
  for (const [h, top, sel] of [["#/library", "library", ".sections a"], ["#/albums/alphabeticalByName", "albums", ".grid .card"], ["#/artists", "artists", ".grid .card"], ["#/genres", "genres", ".genres li"], ["#/songs", "songs", ".songs li"], [`#/artist/${firstArtist}`, "artist", ".grid .card"]]) {
    await hash(h, top);
    counts[top] = await page.$$eval(sel, (e) => e.length);
    expect(counts[top] > 0 && !(await facts()).pageError, `${top} lists something`);
  }
  await hash("#/search/light", "search");
  await page.waitForFunction(() => window.nori.searched?.query === "light", { timeout: 30000 });
  report.browse = { counts, search: (await facts()).searched };
  expect(report.browse.search.songs > 0, "a search finds songs");
}

/** Plays album `id` from song `at` with a click, as a person would. */
async function playAlbum(id, at = 0) {
  await hash(`#/album/${id}`, "album");
  const t0 = Date.now();
  await (await page.$$(".songs li"))[at].click();
  // The player module answers once the first Play has started it.
  await page.waitForFunction(async (a) => window.nori.playerSwitchMs !== undefined && JSON.parse(await window.ask("now"))?.song?.albumId === a && (window.nori.heard?.seconds ?? 0) > 0, { timeout: 60000, polling: 50 }, id);
  return Date.now() - t0;
}

// The player module is loaded in the background once the first page shows, before any Play.
await page.waitForFunction(() => window.nori.playerReadyAt !== undefined, { timeout: 60000 });
const loaded = await facts();
report.player = { early: loaded.playerEarly, readyAtMs: loaded.playerReadyAt, switchMs: loaded.playerSwitchMs, heardBefore: loaded.heard?.seconds ?? 0 };
expect(loaded.playerEarly && !loaded.heard, "the player worker is up before any Play");

/** After the other phases (one tab owns the databases): a fresh page at `at` (another server, such as
 * `web/serve.py --kbps --files-kbps`, for a slow network) that plays a song as soon as its first page shows, while
 * the player module may still load. */
async function early(at) {
  const p = await browser.newPage();
  await p.setCacheEnabled(false);
  await p.goto(at, { waitUntil: "load" });
  await p.waitForFunction(() => window.nori.coreMs !== undefined, { timeout: 120000 });
  if (await p.$("#login")) await p.click("#login button");
  await p.waitForFunction(() => window.nori.page === "home", { timeout: 120000 });
  const shown = await p.evaluate(() => performance.now());
  const id = (await p.evaluate(async () => JSON.parse(await window.ask("read", JSON.stringify({ kind: "albums", sort: "alphabeticalByName", size: 50, offset: 0 }))).albums.find((a) => a.name === "Second Wind")?.id));
  await p.evaluate((x) => (location.hash = `#/album/${x}`), id);
  await p.waitForSelector(".songs li");
  const before = await p.evaluate(() => window.nori.playerReadyAt);
  const clicked = await p.evaluate(() => performance.now());
  await (await p.$$(".songs li"))[0].click();
  await p.waitForFunction(() => (window.nori.heard?.seconds ?? 0) > 2, { timeout: 120000, polling: 50 });
  const f = await p.evaluate(() => JSON.parse(JSON.stringify(window.nori)));
  const n = JSON.parse(await p.evaluate(() => window.ask("now")));
  const songs = (await p.evaluate(async (x) => JSON.parse(await window.ask("read", JSON.stringify({ kind: "album", id: x }))).songs.length, id));
  await p.close();
  return { playerLoadMs: Math.round(f.playerReadyAt - shown), playedDuringLoad: before === undefined, clickToReadyMs: Math.round(f.playerReadyAt - clicked), queue: n.queue.length, albumSongs: songs, index: n.index, state: n.state };
}

if (!only || only.includes("play")) {
  const firstSoundMs = await playAlbum(album("Playback") ?? albums[0].id);
  const before = (await facts()).heard;
  await wait(10000);
  const after = (await facts()).heard;
  const n = await now();
  expect(firstSoundMs < 1000, "the first Play sounds within a second");
  report.play = { firstSoundMs, song: n.song?.title, codecs: (await facts()).codecs, heardS: Math.round(after.seconds - before.seconds), dry: after.dry - before.dry, underruns: n.underruns };
  expect(after.seconds - before.seconds > 9 && after.dry === before.dry, "ten seconds play without running dry");
}

if (!only || only.includes("lyrics")) {
  await playAlbum(album("Sung Album"));
  await page.click("#mini");
  await page.waitForFunction(() => window.nori.lyrics, { timeout: 30000 });
  const lit = [];
  for (let i = 0; i < 30; i++) {
    await wait(500);
    lit.push((await facts()).lyricLine ?? null);
  }
  report.lyrics = { lyrics: (await facts()).lyrics, lit: [...new Set(lit)] };
  expect(report.lyrics.lit.filter((l) => l >= 0).length >= 2, "synced lyrics move on line by line");
  await page.click(".grabber");
}

if (!only || only.includes("skip")) {
  await playAlbum(album("Second Wind", "Alpha Waves"));
  await wait(1500);
  const a = await now();
  await ask("next");
  await wait(1500);
  const b = await now();
  await ask("previous");
  await wait(1500);
  const c = await now();
  report.skip = { start: a.index, afterNext: b.index, afterPrevious: c.index };
  expect(a.index === 0 && b.index === 1 && c.index === 0, "next and previous move through the queue");
}

/** The last song of one album and the first of another (an album's songs play gaplessly), listened to
 * through the transition from 16 s before the first one's end. With `measured`, the seek waits for the
 * plan out of the first song to be made from both songs' measurements (they are measured as they arrive). */
async function transition(name, settings, measured = false) {
  for (const [k, v] of settings) await ask("setting", k, v);
  const a = (await read({ kind: "album", id: album("First Light", "Alpha Waves") })).songs;
  const b = (await read({ kind: "album", id: album("Second Wind", "Beta Band") })).songs;
  const songs = [a[a.length - 1], b[0]];
  await ask("play", JSON.stringify(songs), 0, false, "", "");
  await page.waitForFunction(async (id) => JSON.parse(await window.ask("now"))?.song?.id === id, { timeout: 30000, polling: 250 }, songs[0].id);
  const t0 = Date.now();
  if (measured) {
    const unmeasured = ["Gapless", "EqualPowerFade"];
    await page.waitForFunction(async (u) => {
      const t = JSON.parse(await window.ask("now"))?.transition;
      return t && !u.includes(t.kind);
    }, { timeout: 30000, polling: 250 }, unmeasured).catch(() => {});
  } else await wait(1000);
  const first = await now();
  const plan = first.transition && { ...first.transition, afterMs: Date.now() - t0 };
  const before = (await facts()).heard;
  await ask("seek", first.song.duration * 1000 - 16000);
  const seekHeard = (await facts()).heard.seconds;
  let movedHeard;
  let mixed = 0;
  let moved = false;
  for (let i = 0; i < 40; i++) {
    await wait(500);
    const n = await now();
    if (n.mixing) mixed += 0.5;
    if (n.index === 1 && !moved) movedHeard = (await facts()).heard.seconds;
    if (n.index === 1) moved = true;
  }
  const after = (await facts()).heard;
  const gaps = after.silences.filter(([at]) => at >= before.seconds);
  report[name] = { mixedS: mixed, moved, dry: after.dry - before.dry, silences: gaps, plan, beforeS: before.seconds, seekS: seekHeard, movedS: movedHeard };
  expect(mixed >= 2 && moved && after.dry === before.dry && gaps.length === 0, `a ${name} transition is heard, without a gap`);
  if (measured) expect(plan && !["Gapless", "EqualPowerFade"].includes(plan.kind), `the ${name} transition is planned from both songs' measurements`);
}

// AutoMix first: songs a crossfade just played are still held, and so not measured as they come again.
if (!only || only.includes("automix")) await transition("automix", [["crossfadeSec", "0"], ["autoMix", "true"]], true);
if (!only || only.includes("crossfade")) await transition("crossfade", [["autoMix", "false"], ["crossfadeSec", "6"]]);
await ask("setting", "autoMix", "false");

if (!only || only.includes("codecs")) {
  // Apple Lossless: no browser decodes it, so the Rust decoders are loaded beside the player module.
  await playAlbum(album("Lossless", "Nori Codecs"));
  const before = (await facts()).heard;
  await wait(8000);
  const [after, n] = [(await facts()).heard, await now()];
  report.codecs = { song: n.song?.title, index: n.index, state: n.state, heardS: Math.round(after.seconds - before.seconds), dry: after.dry - before.dry, peak: after.peak };
  expect(n.song?.title === "Lossless Song" && after.seconds - before.seconds > 7 && after.dry === before.dry && after.peak > 0.01, "an Apple Lossless song plays");
}

if (!only || only.includes("speed")) {
  await playAlbum(album("Second Wind", "Alpha Waves"));
  report.speed = {};
  for (const [speed, pitch] of [["1.5", "1"], ["0.75", "1"], ["1", "1.1"]]) {
    await ask("setting", "speed", speed);
    await ask("setting", "pitch", pitch);
    await wait(1500);
    const [h0, a] = [(await facts()).heard, await now()];
    await wait(4000);
    const [h1, b] = [(await facts()).heard, await now()];
    const rate = (b.positionMs - a.positionMs) / 4000;
    report.speed[`${speed}x pitch ${pitch}`] = { rate: Math.round(rate * 100) / 100, dry: h1.dry - h0.dry };
    expect(Math.abs(rate - Number(speed)) < 0.05 && h1.dry === h0.dry, `speed ${speed} and pitch ${pitch} play at their pace`);
  }
  await ask("setting", "speed", "1");
  await ask("setting", "pitch", "1");
}

if (!only || only.includes("edit")) {
  // Playlists made, filled, renamed, reordered, emptied and deleted, and hearts, through the pages' own
  // menus; what the server then holds is read back.
  page.on("dialog", (d) => void d.accept());
  const menu = async (label) => {
    await page.waitForSelector("dialog.menu .menu-item");
    for (const b of await page.$$("dialog.menu .menu-item")) if ((await b.evaluate((x) => x.textContent)) === label) return b.click();
    throw new Error(`no menu item ${label}`);
  };
  const named = async (text) => {
    await page.waitForSelector("dialog.menu input");
    await page.$eval("dialog.menu input", (i) => (i.value = ""));
    await page.type("dialog.menu input", text);
    await page.keyboard.press("Enter");
  };
  const playlistNamed = async (name) => (await read({ kind: "playlists" })).playlists.find((p) => p.name === name);
  const name = `Check ${Date.now() % 100000}`;
  await hash("#/playlists", "playlists");
  await page.click(".page-head .pill");
  await named(name);
  await page.waitForFunction(async (n) => JSON.parse(await window.ask("read", JSON.stringify({ kind: "playlists" }))).playlists.some((p) => p.name === n), { timeout: 30000, polling: 250 }, name);
  const made = await playlistNamed(name);
  // Three songs of an album, added from each one's menu.
  const blue = album("Blue Hour");
  await hash(`#/album/${blue}`, "album");
  for (let i = 0; i < 3; i++) {
    await (await page.$$(".songs li button[aria-label='More']"))[i].click();
    await menu("Add to Playlist");
    await menu(name);
    await page.waitForFunction(async ([id, n]) => JSON.parse(await window.ask("read", JSON.stringify({ kind: "playlist", id }))).songs.length === n, { timeout: 30000, polling: 250 }, [made.id, i + 1]);
  }
  const songsOf = async () => (await read({ kind: "playlist", id: made.id })).songs.map((s) => s.title);
  const added = await songsOf();
  await hash(`#/playlist/${made.id}`, "playlist");
  await page.click(".hero-actions button[aria-label='More']");
  await menu("Rename");
  await named(`${name} renamed`);
  await page.waitForFunction(async (n) => JSON.parse(await window.ask("read", JSON.stringify({ kind: "playlists" }))).playlists.some((p) => p.name === n), { timeout: 30000, polling: 250 }, `${name} renamed`);
  // The page is drawn again with the new name: its rows before that are gone.
  await page.waitForFunction((n) => document.querySelector(".hero-text h1")?.textContent === n, { timeout: 30000, polling: 100 }, `${name} renamed`);
  await page.waitForSelector(".songs li");
  await (await page.$$(".songs li button[aria-label='More']"))[0].click();
  await menu("Move Down");
  await page.waitForFunction(async ([id, t]) => JSON.parse(await window.ask("read", JSON.stringify({ kind: "playlist", id }))).songs[1]?.title === t, { timeout: 30000, polling: 250 }, [made.id, added[0]]);
  const reordered = await songsOf();
  await page.waitForSelector(".songs li");
  await (await page.$$(".songs li button[aria-label='More']"))[2].click();
  await menu("Remove from Playlist");
  await page.waitForFunction(async (id) => JSON.parse(await window.ask("read", JSON.stringify({ kind: "playlist", id }))).songs.length === 2, { timeout: 30000, polling: 250 }, made.id);
  const removed = await songsOf();
  // A heart on a song, seen on the Favorites page, and taken off again.
  const first = (await read({ kind: "album", id: blue })).songs[0];
  await hash(`#/album/${blue}`, "album");
  await (await page.$$(".songs li button[aria-label='More']"))[0].click();
  await menu(first.starred ? "Undo Favorite" : "Favorite");
  await hash("#/favorites", "favorites");
  await page.waitForFunction(async (t) => JSON.parse(await window.ask("read", JSON.stringify({ kind: "starred" }))).songs.some((s) => s.title === t), { timeout: 30000, polling: 250 }, first.title);
  const favorited = await page.$$eval(".songs li .title", (e, t) => e.some((x) => x.textContent === t), first.title);
  await hash(`#/album/${blue}`, "album");
  await (await page.$$(".songs li button[aria-label='More']"))[0].click();
  await menu("Undo Favorite");
  await page.waitForFunction(async (t) => !JSON.parse(await window.ask("read", JSON.stringify({ kind: "starred" }))).songs.some((s) => s.title === t), { timeout: 30000, polling: 250 }, first.title);
  // The playlist deleted.
  await hash(`#/playlist/${made.id}`, "playlist");
  await page.click(".hero-actions button[aria-label='More']");
  await menu("Delete Playlist");
  await page.waitForFunction(async (id) => !JSON.parse(await window.ask("read", JSON.stringify({ kind: "playlists" }))).playlists.some((p) => p.id === id), { timeout: 30000, polling: 250 }, made.id);
  report.edit = { added, reordered, removed, favorited, deleted: true };
  expect(added.length === 3 && reordered[1] === added[0] && reordered[0] === added[1] && removed.length === 2 && favorited, "playlists are made, filled, renamed, reordered, emptied and deleted, and hearts show on Favorites");
}

if (!only || only.includes("crash")) {
  // A panic aborts the player's module: the worker goes, another starts over the saved session, at the
  // song and place this page saw last, playing on.
  await playAlbum(album("Second Wind", "Alpha Waves"));
  await wait(3000);
  const was = await now();
  let w;
  for (const x of page.workers()) if (await x.evaluate(() => self.name === "player")) w = x;
  await w.evaluate(() => setTimeout(() => {
    throw new WebAssembly.RuntimeError("unreachable");
  }, 0));
  await page.waitForFunction(() => window.nori.crashes === 1 && window.nori.restartedAt, { timeout: 30000 });
  await page.waitForFunction(async () => JSON.parse(await window.ask("now")).positionMs > window.nori.restartedAt.ms + 1500, { timeout: 30000, polling: 250 });
  const back = await now();
  report.crash = { song: [was.song?.title, back.song?.title], ms: [was.positionMs, back.positionMs], state: back.state, said: (await facts()).restartedAt };
  expect(back.song?.id === was.song?.id && back.positionMs >= was.positionMs && back.state === "Playing", "after a crash the player plays on at the song and place it was");
}

if (!only || only.includes("tabs")) {
  // One tab owns the databases: a second shows that nori is open elsewhere, and takes it over on asking.
  const second = await browser.newPage();
  await second.goto(url, { waitUntil: "load" });
  await second.waitForFunction(() => window.nori.elsewhere, { timeout: 30000 });
  await second.click("#use-here");
  // A tab in the background draws no frames: polled by time.
  await page.waitForFunction(() => window.nori.movedAway, { timeout: 30000, polling: 250 });
  await second.waitForFunction(() => window.nori.page === "home", { timeout: 60000 });
  const stored = await second.evaluate(() => window.nori.stored);
  report.tabs = { secondSawElsewhere: true, firstMovedAway: true, secondHome: true, secondStored: stored };
  expect(stored, "the tab that took over keeps the databases in OPFS");
  await second.close();
  await page.bringToFront();
  // The closed tab lets the databases go a moment later.
  await page.evaluate(() => navigator.locks.request("nori-databases", () => {}));
  await page.reload({ waitUntil: "load" });
  await page.waitForFunction(() => window.nori.page !== undefined, { timeout: 60000 });
}

if (!only || only.includes("offline")) {
  // The service worker keeps the shell and modules: a reload comes from it, and so does a start without
  // the network, the library from the databases. Nothing of the API or audio is kept.
  await page.waitForFunction(() => window.nori.serviceWorker, { timeout: 30000, polling: 250 });
  await page.evaluate(() => (location.hash = "#/"));
  await page.reload({ waitUntil: "load" });
  await page.waitForFunction(() => navigator.serviceWorker.controller !== null && window.nori.page === "home", { timeout: 60000, polling: 250 });
  const reload = async () => {
    const t0 = Date.now();
    await page.reload({ waitUntil: "load" });
    await page.waitForFunction(() => window.nori.page === "home" && window.nori.playerReadyAt !== undefined, { timeout: 60000, polling: 50 });
    return Date.now() - t0;
  };
  const cachedMs = await reload();
  await page.setOfflineMode(true);
  const offlineMs = await reload();
  const offline = await facts();
  await page.setOfflineMode(false);
  const kept = await page.evaluate(async () => {
    const all = [];
    for (const k of await caches.keys()) for (const r of await (await caches.open(k)).keys()) all.push(new URL(r.url).pathname);
    return all;
  });
  report.offline = { cachedReloadMs: cachedMs, offlineReloadMs: offlineMs, offlineHomeAlbums: offline.albums, kept: kept.length, api: kept.filter((u) => u.includes("/rest/") || u.includes("/nori/")).length };
  expect(offline.albums > 0 && report.offline.api === 0 && kept.some((u) => u.endsWith("player.wasm")), "without the network the client starts from its service worker, the library from its databases, and keeps no API answers");

  // A new build (here: web/dist's service worker under another hash) installs beside the running one
  // and takes over at the next load, its old files gone.
  const swFile = new URL("../dist/sw.js", import.meta.url);
  if (fs.existsSync(swFile)) {
    const was = fs.readFileSync(swFile, "utf8");
    const build = was.match(/const BUILD = "([0-9a-f]+)"/)[1];
    fs.writeFileSync(swFile, was.replace(`const BUILD = "${build}"`, `const BUILD = "${build}0"`));
    try {
      await page.evaluate(async () => (await navigator.serviceWorker.getRegistration()).update());
      await page.waitForFunction(async () => !!(await navigator.serviceWorker.getRegistration()).waiting, { timeout: 30000, polling: 250 });
      const before = await page.evaluate(() => caches.keys());
      await page.reload({ waitUntil: "load" });
      await page.waitForFunction(() => window.nori.page !== undefined, { timeout: 60000, polling: 250 });
      const after = await page.evaluate(() => caches.keys());
      report.update = { before, after };
      expect(after.length === 1 && after[0] === `nori-${build}0`, "a new build takes over at the next load, the old one's files gone");
    } finally {
      fs.writeFileSync(swFile, was);
    }
  }
}

if (!only || only.includes("early")) {
  await page.close();
  report.early = await early(url);
  expect(report.early.state === "Playing" && report.early.index === 0 && report.early.queue >= report.early.albumSongs && report.early.queue < 2 * report.early.albumSongs, "a Play pressed while the player loads plays once");
  if (slow) {
    report.earlySlow = await early(slow);
    expect(report.earlySlow.playedDuringLoad && report.earlySlow.state === "Playing" && report.earlySlow.queue < 2 * report.earlySlow.albumSongs, "on a slow network a Play pressed while the player loads waits for it and plays once");
  }
}

/** A fresh page at `at` (`web/serve.py --kbps 400`: songs arriving at 400 kbit/s) plays the Playback
 * album: the time from the click to the first sound, and 20 s heard after it. */
async function slowStart(at) {
  const p = await browser.newPage();
  await p.goto(at, { waitUntil: "load" });
  await p.waitForFunction(() => window.nori.coreMs !== undefined, { timeout: 60000 });
  if (await p.$("#login")) await p.click("#login button");
  await p.waitForFunction(() => window.nori.page === "home", { timeout: 60000 });
  const id = await p.evaluate(async () => JSON.parse(await window.ask("read", JSON.stringify({ kind: "albums", sort: "alphabeticalByName", size: 100, offset: 0 }))).albums.find((a) => a.name === "Playback")?.id);
  await p.evaluate((x) => (location.hash = `#/album/${x}`), id);
  await p.waitForSelector(".songs li");
  const t0 = Date.now();
  await (await p.$$(".songs li"))[0].click();
  await p.waitForFunction(() => (window.nori.heard?.seconds ?? 0) > 0, { timeout: 60000, polling: 20 });
  const firstSoundMs = Date.now() - t0;
  const before = await p.evaluate(() => JSON.parse(JSON.stringify(window.nori.heard)));
  await new Promise((r) => setTimeout(r, 20000));
  const after = await p.evaluate(() => JSON.parse(JSON.stringify(window.nori.heard)));
  const n = JSON.parse(await p.evaluate(() => window.ask("now")));
  await p.close();
  return { firstSoundMs, song: n.song?.title, heardS: Math.round(after.seconds - before.seconds), dry: after.dry - before.dry, underruns: n.underruns };
}

// A second address of the client whose songs arrive at 400 kbit/s (`web/serve.py --port 8082 --kbps 400`).
const throttled = process.env.THROTTLED_URL;
if (throttled && (!only || only.includes("slowstart"))) {
  report.slowStart = await slowStart(throttled);
  expect(report.slowStart.firstSoundMs < 2500 && report.slowStart.dry === 0 && report.slowStart.heardS >= 19, "at 400 kbit/s a song sounds within 2.5 s and plays on without running dry");
}

report.failures = failures;
console.log(JSON.stringify(report, null, 1));
if (logs.length) console.log(logs.slice(0, 40).join("\n"));
await browser.close();
process.exit(failures.length ? 1 : 0);
