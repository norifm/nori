// The web client in headless Chrome against the local Navidrome (tools/dev-server.sh) through
// web/serve.py: sign-in, browsing, search, a song streamed and played, next and previous, synced lyrics,
// a crossfade and an AutoMix transition heard as continuous samples, speed and pitch, the seek bar
// running and holding, a click on a lyric line, how soon a pause is heard. Exits non-zero when
// something is off, and prints what it saw.
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
// What the page keeps, and what the sound card heard (null before there is one).
const facts = () => page.evaluate(async () => ({ ...JSON.parse(JSON.stringify(window.nori)), heard: (await window.nori.heard?.()) ?? null }));
const ask = (call, ...args) => page.evaluate((c, a) => window.ask(c, ...a), call, args);
const now = async () => JSON.parse(await ask("now"));
const read = async (q) => JSON.parse(await ask("read", JSON.stringify(q)));
const report = {};
const failures = [];
const expect = (ok, what) => ok || failures.push(what);

/** Until `p` (this page by default) has heard `s` more seconds than `from` (a `nori.heard`), 60 s at
 * most; the time is the music's, not the clock's. */
const heardFor = (s, from, p = page) => p.waitForFunction(async (x, f) => ((await window.nori.heard?.())?.seconds ?? 0) >= f + x, { timeout: 60000, polling: 100 }, s, from?.seconds ?? 0).catch(() => {});
/** Until the queue's index is `i`, 10 s at most. */
const atIndex = (i) => page.waitForFunction(async (x) => JSON.parse(await window.ask("now"))?.index === x, { timeout: 10000, polling: 100 }, i).catch(() => {});

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
  for (const [h, top, sel] of [["#/library", "library", ".chips .chip"], ["#/library/Albums", "library", ".grid .card"], ["#/library/Artists", "library", ".rows .nav-row"], ["#/library/Genres", "library", ".genres li"], ["#/library/Songs", "library", ".songs li"], [`#/artist/${firstArtist}`, "artist", ".card"], ["#/settings", "settings", "[data-page]"], ["#/settings/Playing", "settings", "[data-setting]"]]) {
    await hash(h, top);
    // The library's sections read themselves once drawn.
    await page.waitForSelector(sel, { timeout: 30000 }).catch(() => {});
    counts[h] = await page.$$eval(sel, (e) => e.length);
    expect(counts[h] > 0 && !(await facts()).pageError, `${h} lists something`);
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
  await (await page.$$(".songs li .play"))[at].click();
  // The player module answers once the first Play has started it.
  await page.waitForFunction(async (a) => window.nori.playerSwitchMs !== undefined && JSON.parse(await window.ask("now"))?.song?.albumId === a && ((await window.nori.heard?.())?.seconds ?? 0) > 0, { timeout: 60000, polling: 50 }, id);
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
  p.on("pageerror", (e) => logs.push(`early pageerror: ${e}`));
  await p.setViewport({ width: 390, height: 844, isMobile: true, hasTouch: true });
  await p.setCacheEnabled(false);
  await p.goto(at, { waitUntil: "load" });
  await p.waitForFunction(() => window.nori.coreMs !== undefined, { timeout: 120000 });
  if (await p.$("#login")) await p.click("#login button");
  await p.waitForFunction(() => window.nori.page === "home", { timeout: 120000 });
  const shown = await p.evaluate(() => performance.now());
  const id = (await p.evaluate(async () => JSON.parse(await window.ask("read", JSON.stringify({ kind: "albums", sort: "alphabeticalByName", size: 50, offset: 0 }))).albums.find((a) => a.name === "Second Wind")?.id));
  await p.evaluate((x) => (location.hash = `#/album/${x}`), id);
  await p.waitForSelector(".songs li .play");
  // A restored queue already shows the fixed mini-player on a phone.
  await p.$eval(".songs li .play", (e) => e.scrollIntoView({ block: "center" }));
  const before = await p.evaluate(() => window.nori.playerReadyAt);
  const clicked = await p.evaluate(() => performance.now());
  await (await p.$$(".songs li .play"))[0].click();
  await p.waitForFunction(async () => ((await window.nori.heard?.())?.seconds ?? 0) > 2, { timeout: 120000, polling: 50 }).catch(async (e) => {
    console.error("early playback", await p.evaluate(async () => {
      const n = JSON.parse(await window.ask("now"));
      const f = window.nori;
      return { playerReadyAt: f.playerReadyAt, heard: await f.heard?.(), error: f.error, crashes: f.crashes, state: n.state, positionMs: n.positionMs, queue: n.queue.length, song: n.song?.title };
    }), logs);
    throw e;
  });
  const f = await p.evaluate(() => JSON.parse(JSON.stringify(window.nori)));
  const n = JSON.parse(await p.evaluate(() => window.ask("now")));
  const songs = (await p.evaluate(async (x) => JSON.parse(await window.ask("read", JSON.stringify({ kind: "album", id: x }))).songs.length, id));
  await p.close();
  return { playerLoadMs: Math.round(f.playerReadyAt - shown), playedDuringLoad: before === undefined, clickToReadyMs: Math.round(f.playerReadyAt - clicked), queue: n.queue.length, albumSongs: songs, index: n.index, state: n.state };
}

if (!only || only.includes("play")) {
  const firstSoundMs = await playAlbum(album("Playback") ?? albums[0].id);
  const before = (await facts()).heard;
  await heardFor(10, before);
  const after = (await facts()).heard;
  const n = await now();
  expect(firstSoundMs < 1000, "the first Play sounds within a second");
  report.play = { firstSoundMs, song: n.song?.title, codecs: (await facts()).codecs, heardS: Math.round(after.seconds - before.seconds), dry: after.dry - before.dry, underruns: n.underruns };
  expect(after.seconds - before.seconds > 9 && after.dry === before.dry, "ten seconds play without running dry");
}

/** Opens `name` (the lyrics or the queue in the player, the devices' sheet) from the player bar. */
async function pane(name, p = page) {
  if (!(await p.$(`#${name}`))) await p.click(`#bar [data-pane=${name}]`);
  await p.waitForSelector(`#${name}`);
}

if (!only || only.includes("lyrics")) {
  await playAlbum(album("Sung Album"));
  await pane("lyrics");
  await page.waitForFunction(() => window.nori.lyrics, { timeout: 30000 });
  // The lines lit as the song plays, until two have been.
  await page.waitForFunction(() => {
    const lit = (window.litLines ??= new Set());
    lit.add(window.nori.lyricLine ?? null);
    return [...lit].filter((l) => l >= 0).length >= 2;
  }, { timeout: 30000, polling: 100 }).catch(() => {});
  const lit = await page.evaluate(() => [...window.litLines]);
  report.lyrics = { lyrics: (await facts()).lyrics, lit };
  expect(report.lyrics.lit.filter((l) => l >= 0).length >= 2, "synced lyrics move on line by line");
  // A later line clicked lights at once and plays from where it is sung (the lyrics' timing applied);
  // Enter on a focused line does the same.
  const lines = report.lyrics.lyrics.lines;
  const later = Math.min(Math.max(...lit) + 4, lines - 2);
  const tapped = await page.evaluate((k) => {
    const [t, at] = [window.nori.audioTime, performance.now()];
    document.querySelector(`.lyrics .line[data-i="${k}"]`).click();
    return { t, at };
  }, later);
  await page.waitForFunction((k) => window.nori.lyricLine === k, { timeout: 2000, polling: "raf" }, later).catch(() => {});
  const litMs = Math.round((await page.evaluate(() => performance.now())) - tapped.at);
  await page.waitForFunction(async (t) => ((await window.nori.heard?.())?.freshAt ?? 0) > t, { timeout: 5000, polling: 20 }, tapped.t).catch(() => {});
  const tapHeard = (await facts()).heard.freshAt;
  await new Promise((r) => setTimeout(r, 1000));
  const [landed, at, sought] = await page.evaluate(async () => [JSON.parse(await window.ask("now")).positionMs, window.nori.audioTime, window.nori.tapped?.ms]);
  const enter = later + 1;
  await page.focus(`.lyrics .line[data-i="${enter}"]`);
  await page.keyboard.press("Enter");
  await page.waitForFunction((k) => window.nori.lyricLine === k, { timeout: 2000, polling: 50 }, enter).catch(() => {});
  report.lyrics.tap = { line: later, litMs, heardMs: Math.round((tapHeard - tapped.t) * 1000), soughtMs: sought, placeMs: Math.round(landed - (sought + (at - tapHeard) * 1000)), enterLit: (await facts()).lyricLine === enter };
  const tap = report.lyrics.tap;
  expect(tap.litMs < 100 && tap.heardMs >= 0 && tap.heardMs < 100 && Math.abs(tap.placeMs) < 100 && tap.enterLit, "a lyric line clicked or entered lights at once and is heard from where it is sung");
  await page.click("#full [aria-label=Close]");
}

if (!only || only.includes("skip")) {
  await playAlbum(album("Second Wind", "Alpha Waves"));
  const a = await now();
  await ask("next");
  await atIndex(1);
  const b = await now();
  await ask("previous");
  await atIndex(0);
  const c = await now();
  report.skip = { start: a.index, afterNext: b.index, afterPrevious: c.index };
  expect(a.index === 0 && b.index === 1 && c.index === 0, "next and previous move through the queue");
}

if (!only || only.includes("instant")) {
  // Pause, play, seek and next act on what is heard at once, on the sound card's clock: pause is silent
  // within 50 ms and play sounds again as soon, a seek and a skip are heard from their place, and the
  // place runs on with the music across a pause (none of it lost or played twice).
  await playAlbum(album("Second Wind", "Alpha Waves"));
  await heardFor(3, (await facts()).heard);
  const press = (sel) => page.evaluate((s) => {
    const t = window.nori.audioTime;
    document.querySelector(s).click();
    return t;
  }, sel);
  const settle = (ms) => new Promise((r) => setTimeout(r, ms));
  // The place less the music heard, ms: the same as long as the place follows the music.
  const offset = async () => {
    const all = [];
    for (let i = 0; i < 5; i++) {
      const [n, f] = await page.evaluate(async () => [JSON.parse(await window.ask("now")).positionMs, (await window.nori.heard()).seconds]);
      all.push(n - f * 1000);
      await settle(120);
    }
    return all.sort((a, b) => a - b)[2];
  };
  const ms = (s) => Math.round(s * 1000);
  // The seek bar and its times as shown, and the core's place.
  const shown = () => page.evaluate(async () => {
    const n = JSON.parse(await window.ask("now"));
    const run = document.querySelector("#bar .run");
    const bar = Math.round((1 + new DOMMatrix(getComputedStyle(run).transform).m41 / run.getBoundingClientRect().width) * n.song.duration * 1000);
    return { bar, core: n.positionMs, at: document.querySelector("#bar .seek .time:first-child").textContent, left: document.querySelector("#bar .seek .time:last-child").textContent, durationMs: n.song.duration * 1000 };
  });
  const clock = (x) => `${Math.floor(x / 60000)}:${String(Math.floor(x / 1000) % 60).padStart(2, "0")}`;
  const s0 = await shown();
  await settle(2000);
  const s1 = await shown();
  const bar = { movedMs: s1.bar - s0.bar, offCoreMs: s1.bar - s1.core, at: [s0.at, s1.at], left: [s0.left, s1.left] };
  expect(Math.abs(bar.movedMs - 2000) < 250 && Math.abs(bar.offCoreMs) < 250 && s1.at === clock(s1.bar) && s1.left === `-${clock(s1.durationMs - s1.bar)}` && s1.left !== s0.left, "while playing the seek bar and its times move with the core's place");
  const before = await offset();
  const paused = await press("#bar [aria-label=Pause]");
  await settle(800);
  const p0 = (await facts()).heard;
  const held0 = (await now()).positionMs;
  const b0 = await shown();
  await settle(500);
  const held1 = (await now()).positionMs;
  const b1 = await shown();
  bar.pausedMovedMs = b1.bar - b0.bar;
  bar.pausedOffCoreMs = b1.bar - b1.core;
  expect(b1.bar === b0.bar && b1.at === b0.at && Math.abs(bar.pausedOffCoreMs) < 250, "paused, the seek bar and its times stay at the core's place");
  const pauseMs = ms(p0.lastLoud - paused);
  const played = await press("#bar [aria-label=Play]");
  await page.waitForFunction(async (t) => ((await window.nori.heard?.())?.soundAt ?? 0) > t, { timeout: 5000, polling: 20 }, played).catch(() => {});
  const resumeMs = ms((await facts()).heard.soundAt - played);
  await heardFor(1.5, (await facts()).heard);
  const after = await offset();
  const seekTo = 20000;
  const sought = await page.evaluate((to) => {
    const t = window.nori.audioTime;
    const seek = document.querySelector("#bar .seek input");
    seek.value = String(to);
    seek.dispatchEvent(new Event("change", { bubbles: true }));
    return t;
  }, seekTo);
  await settle(150);
  const s2 = await shown();
  bar.afterSeekMs = s2.bar - seekTo;
  expect(Math.abs(bar.afterSeekMs) < 250, "after a seek the seek bar is at once at its place");
  await page.waitForFunction(async (t) => ((await window.nori.heard?.())?.freshAt ?? 0) > t, { timeout: 5000, polling: 20 }, sought).catch(() => {});
  const seekHeard = (await facts()).heard.freshAt;
  await settle(1000);
  const [landed, at] = await page.evaluate(async () => [JSON.parse(await window.ask("now")).positionMs, window.nori.audioTime]);
  const seekPlaceMs = Math.round(landed - (seekTo + (at - seekHeard) * 1000));
  const skipped = await press("#bar [aria-label=Next]");
  await page.waitForFunction(async (t) => ((await window.nori.heard?.())?.freshAt ?? 0) > t, { timeout: 5000, polling: 20 }, skipped).catch(() => {});
  const nextMs = ms((await facts()).heard.freshAt - skipped);
  const index = (await now()).index;
  report.instant = { pauseMs, resumeMs, seekMs: ms(seekHeard - sought), nextMs, pausedMovedMs: held1 - held0, offsetMs: [Math.round(before), Math.round(after)], seekPlaceMs, index, seekBar: bar };
  expect(pauseMs >= 0 && pauseMs < 50, "a pause is silent within 50 ms");
  expect(resumeMs >= 0 && resumeMs < 50, "play sounds again within 50 ms");
  expect(report.instant.seekMs >= 0 && report.instant.seekMs < 100 && Math.abs(seekPlaceMs) < 100, "a seek is heard from its place within 100 ms");
  expect(nextMs >= 0 && nextMs < 250 && index === 1, "next is heard within 250 ms");
  expect(held1 === held0 && Math.abs(after - before) < 40, "the place stays still while paused and follows the music across it");
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
  // The plan out of the first song; measured, one made from both songs' measurements.
  const unmeasured = measured ? ["Gapless", "EqualPowerFade"] : [];
  await page.waitForFunction(async (u) => {
    const t = JSON.parse(await window.ask("now"))?.transition;
    return t && !u.includes(t.kind);
  }, { timeout: 30000, polling: 250 }, unmeasured).catch(() => {});
  const first = await now();
  const plan = first.transition && { ...first.transition, afterMs: Date.now() - t0 };
  const before = (await facts()).heard;
  await ask("seek", first.song.duration * 1000 - 16000);
  const seekHeard = (await facts()).heard.seconds;
  // Listened to until the second song plays on its own: how long the two mixed, and when it took over.
  await page.waitForFunction(async () => {
    const s = (window.through ??= { mixedMs: 0, last: performance.now(), moved: false });
    const n = JSON.parse(await window.ask("now"));
    const t = performance.now();
    if (n.mixing) s.mixedMs += t - s.last;
    s.last = t;
    if (n.index === 1 && !s.moved) [s.moved, s.movedHeard] = [true, (await window.nori.heard()).seconds];
    return s.moved && !n.mixing;
  }, { timeout: 30000, polling: 250 }).catch(() => {});
  const { mixedMs, moved, movedHeard } = await page.evaluate(() => window.through);
  await page.evaluate(() => (window.through = undefined));
  const mixed = mixedMs / 1000;
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
  await heardFor(8, before);
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
    // Taken up once the sound heard has run on a while; then the song's pace against the sound's.
    await heardFor(1.5, (await facts()).heard);
    const [h0, a] = [(await facts()).heard, await now()];
    await heardFor(4, h0);
    const [h1, b] = [(await facts()).heard, await now()];
    const rate = (b.positionMs - a.positionMs) / ((h1.seconds - h0.seconds) * 1000);
    report.speed[`${speed}x pitch ${pitch}`] = { rate: Math.round(rate * 100) / 100, dry: h1.dry - h0.dry };
    expect(Math.abs(rate - Number(speed)) < 0.05 && h1.dry === h0.dry, `speed ${speed} and pitch ${pitch} play at their pace`);
  }
  await ask("setting", "speed", "1");
  await ask("setting", "pitch", "1");
}

/** The seek bar's run, 0 to 1, as the browser draws it now. */
const bar = (p = page) =>
  p.$eval("#bar .run", (e) => {
    const m = new DOMMatrix(getComputedStyle(e).transform);
    return 1 + m.m41 / e.getBoundingClientRect().width;
  });
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
const stateIs = (s) => page.waitForFunction(async (x) => JSON.parse(await window.ask("now")).state === x, { timeout: 10000, polling: 50 }, s);

if (!only || only.includes("seekbar")) {
  // The bar runs on by itself while the song plays, holds while it is paused, and no timer runs while
  // it holds or the page is hidden.
  await playAlbum(album("Second Wind", "Alpha Waves"));
  await heardFor(1, (await facts()).heard);
  const a0 = await bar();
  await wait(1500);
  const a1 = await bar();
  const playingTimers = (await facts()).clockTimers;
  await page.click("#bar [aria-label=Pause]");
  await stateIs("Paused");
  await wait(300);
  const p0 = await bar();
  await wait(1500);
  const p1 = await bar();
  const pausedTimers = (await facts()).clockTimers;
  await page.click("#bar [aria-label=Play]");
  await stateIs("Playing");
  // Hidden: the times stop; seen again, they run on.
  await page.evaluate(() => {
    Object.defineProperty(document, "hidden", { configurable: true, get: () => true });
    document.dispatchEvent(new Event("visibilitychange"));
  });
  const hiddenTimers = (await facts()).clockTimers;
  await page.evaluate(() => {
    delete document.hidden;
    document.dispatchEvent(new Event("visibilitychange"));
  });
  const shownTimers = (await facts()).clockTimers;
  const length = (await now()).song.duration * 1000;
  report.seekbar = { playing: [a0, a1], paused: [p0, p1], expectedStep: 1500 / length, timers: { playing: playingTimers, paused: pausedTimers, hidden: hiddenTimers, shown: shownTimers } };
  expect(a1 - a0 > (0.5 * 1500) / length && a1 - a0 < (2 * 1500) / length, "the seek bar runs on while the song plays");
  expect(Math.abs(p1 - p0) < 1e-4, "the seek bar holds while the song is paused");
  expect(playingTimers > 0 && pausedTimers === 0 && hiddenTimers === 0 && shownTimers > 0, "no timer runs while paused or hidden");
}

if (!only || only.includes("edit")) {
  // Playlists made, filled, renamed, reordered, emptied and deleted, and hearts, through the pages' own
  // menus; what the server then holds is read back.
  page.on("dialog", (d) => void d.accept());
  const menu = async (label) => {
    // A song's sheet, or a menu dropped from a button.
    await page.waitForSelector("dialog.menu .menu-item, .popover .popover-item");
    for (const b of await page.$$("dialog.menu .menu-item, .popover .popover-item")) if ((await b.evaluate((x) => x.textContent)) === label) return b.click();
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
  await hash("#/library/Playlists", "library");
  await page.waitForSelector("#new-playlist");
  await page.click("#new-playlist");
  await named(name);
  await page.waitForFunction(async (n) => JSON.parse(await window.ask("read", JSON.stringify({ kind: "playlists" }))).playlists.some((p) => p.name === n), { timeout: 30000, polling: 250 }, name);
  const made = await playlistNamed(name);
  // Three songs of an album, added from each one's menu.
  const blue = album("Blue Hour");
  await hash(`#/album/${blue}`, "album");
  for (let i = 0; i < 3; i++) {
    await (await page.$$(".songs li button[aria-label='More']"))[i].click();
    await menu("Add to playlist…");
    await menu(name);
    await page.waitForFunction(async ([id, n]) => JSON.parse(await window.ask("read", JSON.stringify({ kind: "playlist", id }))).songs.length === n, { timeout: 30000, polling: 250 }, [made.id, i + 1]);
  }
  const songsOf = async () => (await read({ kind: "playlist", id: made.id })).songs.map((s) => s.title);
  const added = await songsOf();
  await hash(`#/playlist/${made.id}`, "playlist");
  await page.click(".hero-page .buttons button[aria-label='More']");
  await menu("Rename");
  await named(`${name} renamed`);
  await page.waitForFunction(async (n) => JSON.parse(await window.ask("read", JSON.stringify({ kind: "playlists" }))).playlists.some((p) => p.name === n), { timeout: 30000, polling: 250 }, `${name} renamed`);
  // The page is drawn again with the new name: its rows before that are gone.
  await page.waitForFunction((n) => document.querySelector(".hero-page h1")?.textContent === n, { timeout: 30000, polling: 100 }, `${name} renamed`);
  await page.waitForSelector(".songs li");
  await (await page.$$(".songs li button[aria-label='More']"))[0].click();
  await menu("Move down");
  await page.waitForFunction(async ([id, t]) => JSON.parse(await window.ask("read", JSON.stringify({ kind: "playlist", id }))).songs[1]?.title === t, { timeout: 30000, polling: 250 }, [made.id, added[0]]);
  const reordered = await songsOf();
  await page.waitForSelector(".songs li");
  await (await page.$$(".songs li button[aria-label='More']"))[2].click();
  await menu("Remove from playlist");
  await page.waitForFunction(async (id) => JSON.parse(await window.ask("read", JSON.stringify({ kind: "playlist", id }))).songs.length === 2, { timeout: 30000, polling: 250 }, made.id);
  const removed = await songsOf();
  // A heart on a song, seen on the Favorites page, and taken off again.
  const first = (await read({ kind: "album", id: blue })).songs[0];
  await hash(`#/album/${blue}`, "album");
  await (await page.$$(".songs li button[aria-label='More']"))[0].click();
  await menu(first.starred ? "Remove from favorites" : "Add to favorites");
  await hash("#/favorites", "favorites");
  await page.waitForFunction(async (t) => JSON.parse(await window.ask("read", JSON.stringify({ kind: "starred" }))).songs.some((s) => s.title === t), { timeout: 30000, polling: 250 }, first.title);
  const favorited = await page.$$eval(".songs li .title", (e, t) => e.some((x) => x.textContent === t), first.title);
  await hash(`#/album/${blue}`, "album");
  await (await page.$$(".songs li button[aria-label='More']"))[0].click();
  await menu("Remove from favorites");
  await page.waitForFunction(async (t) => !JSON.parse(await window.ask("read", JSON.stringify({ kind: "starred" }))).songs.some((s) => s.title === t), { timeout: 30000, polling: 250 }, first.title);
  // The playlist deleted.
  await hash(`#/playlist/${made.id}`, "playlist");
  await page.click(".hero-page .buttons button[aria-label='More']");
  await menu("Delete playlist");
  await page.waitForFunction(async (id) => !JSON.parse(await window.ask("read", JSON.stringify({ kind: "playlists" }))).playlists.some((p) => p.id === id), { timeout: 30000, polling: 250 }, made.id);
  report.edit = { added, reordered, removed, favorited, deleted: true };
  expect(added.length === 3 && reordered[1] === added[0] && reordered[0] === added[1] && removed.length === 2 && favorited, "playlists are made, filled, renamed, reordered, emptied and deleted, and hearts show on Favorites");
}

if (!only || only.includes("crash")) {
  // A panic aborts the player's module: the worker goes, another starts over the saved session, at the
  // song and place this page saw last, playing on.
  await playAlbum(album("Second Wind", "Alpha Waves"));
  await heardFor(3, (await facts()).heard);
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
    const build = (await page.evaluate(() => caches.keys())).find((k) => k.startsWith("nori-")).slice(5);
    fs.writeFileSync(swFile, was.replaceAll(build, `${build}0`));
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
  await p.setViewport({ width: 1280, height: 800 });
  await p.goto(at, { waitUntil: "load" });
  await p.waitForFunction(() => window.nori.coreMs !== undefined, { timeout: 60000 });
  if (await p.$("#login")) await p.click("#login button");
  await p.waitForFunction(() => window.nori.page === "home", { timeout: 60000 });
  const id = await p.evaluate(async () => JSON.parse(await window.ask("read", JSON.stringify({ kind: "albums", sort: "alphabeticalByName", size: 100, offset: 0 }))).albums.find((a) => a.name === "Playback")?.id);
  await p.evaluate((x) => (location.hash = `#/album/${x}`), id);
  await p.waitForSelector(".songs li .play");
  const t0 = Date.now();
  await (await p.$$(".songs li .play"))[0].click();
  await p.waitForFunction(async () => ((await window.nori.heard?.())?.seconds ?? 0) > 0, { timeout: 60000, polling: 20 });
  const firstSoundMs = Date.now() - t0;
  const before = await p.evaluate(() => window.nori.heard());
  await heardFor(20, before, p);
  const after = await p.evaluate(() => window.nori.heard());
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
