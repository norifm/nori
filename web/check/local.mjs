// A local files profile in headless Chrome, with no server (web/serve.py over web/dist): the welcome's
// local files picked, and its Continue choosing a folder of FLAC files ffmpeg makes; then its albums, their
// covers (a folder's picture and one a file holds), search and a song played from its file; after a
// reload the folder is read again without being asked for. A headless browser has no folder picker: a
// folder in the origin's private file system, filled with the files, stands in for the system's. Exits
// non-zero when something is off, and prints what it saw.
//
//   npm --prefix web/check install && node web/check/local.mjs [http://localhost:8080/] [screenshots dir]
//
// CHROME names the browser (default: /usr/bin/chromium); ffmpeg makes the music.
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import puppeteer from "puppeteer-core";

const url = process.argv[2] ?? "http://localhost:8080/";
const shots = process.argv[3];
const failures = [];
const expect = (ok, what) => ok || failures.push(what);
const report = {};

// Two albums: one with its cover beside it, one whose first file holds its picture.
const music = fs.mkdtempSync(path.join(os.tmpdir(), "nori-local-"));
const ffmpeg = (...a) => execFileSync("ffmpeg", ["-v", "error", "-y", ...a]);
const tone = (rel, hz, tags, picture) => {
  const out = path.join(music, rel);
  fs.mkdirSync(path.dirname(out), { recursive: true });
  const meta = Object.entries(tags).flatMap(([k, v]) => ["-metadata", `${k}=${v}`]);
  const art = picture ? ["-i", picture, "-map", "0:a", "-map", "1:v", "-c:v", "png", "-disposition:v", "attached_pic"] : [];
  ffmpeg("-f", "lavfi", "-i", `sine=frequency=${hz}:duration=4`, ...art, ...meta, out);
};
const picture = (out, color) => ffmpeg("-f", "lavfi", "-i", `color=c=${color}:s=300x300`, "-frames:v", "1", out);
fs.mkdirSync(path.join(music, "Ann/First"), { recursive: true });
picture(path.join(music, "Ann/First/cover.jpg"), "0xc0392b");
picture(path.join(music, "held.png"), "0x2e86c1");
const ann = { artist: "Ann", album_artist: "Ann", album: "First", date: "1997", genre: "Jazz" };
tone("Ann/First/01.flac", 440, { ...ann, title: "Opening", track: "1" });
tone("Ann/First/02.flac", 550, { ...ann, title: "Closing", track: "2" });
tone("Cy/Hits/01.flac", 660, { title: "Up", artist: "Cy", album: "Hits", track: "1" }, path.join(music, "held.png"));
tone("Cy/Hits/02.flac", 770, { title: "Down", artist: "Cy", album: "Hits", track: "2" });
fs.rmSync(path.join(music, "held.png"));
const files = [];
const walk = (dir) => fs.readdirSync(dir, { withFileTypes: true }).forEach((e) => (e.isDirectory() ? walk(path.join(dir, e.name)) : files.push(path.relative(music, path.join(dir, e.name)))));
walk(music);

const browser = await puppeteer.launch({
  executablePath: process.env.CHROME ?? "/usr/bin/chromium",
  headless: "new",
  args: ["--no-sandbox", "--autoplay-policy=no-user-gesture-required", "--mute-audio"],
});
const page = await browser.newPage();
await page.setViewport({ width: 1280, height: 800 });
const logs = [];
page.on("pageerror", (e) => logs.push(`pageerror: ${e}`));
page.on("console", (m) => m.type() === "error" && logs.push(`console: ${m.text()}`));
page.on("workercreated", (w) => w.on("console", (m) => m.type() !== "log" && logs.push(`worker ${m.type()}: ${m.text()}`)));
// The system's picker: the folder "Music" of the origin's private file system.
await page.evaluateOnNewDocument(() => {
  window.showDirectoryPicker = async () => (await navigator.storage.getDirectory()).getDirectoryHandle("Music");
});
const ask = (call, ...args) => page.evaluate((c, a) => window.ask(c, ...a), call, args);
const read = async (q) => JSON.parse(await ask("read", JSON.stringify(q)));
const shot = async (name) => shots && (await page.screenshot({ path: path.join(shots, `${name}.png`) }));
const settle = () => new Promise((r) => setTimeout(r, 800));
/** Until `f` (run in the page) is true, `ms` at most. */
const until = (f, ms = 30000, ...a) => page.waitForFunction(f, { timeout: ms, polling: 250 }, ...a);

await page.goto(url, { waitUntil: "load" });
await until(() => document.querySelector("#welcome"));
// The folder the picker gives, filled with the music.
for (const rel of files) {
  const bytes = fs.readFileSync(path.join(music, rel)).toString("base64");
  await page.evaluate(async (p, b) => {
    let dir = await (await navigator.storage.getDirectory()).getDirectoryHandle("Music", { create: true });
    const parts = p.split("/");
    for (const d of parts.slice(0, -1)) dir = await dir.getDirectoryHandle(d, { create: true });
    const w = await (await dir.getFileHandle(parts.at(-1), { create: true })).createWritable();
    await w.write(Uint8Array.from(atob(b), (c) => c.charCodeAt(0)));
    await w.close();
  }, `Music/${rel}`.split("/").slice(1).join("/"), bytes);
}

// The welcome: local files picked, and Continue asks for the folder in the same press.
await page.click("#welcome [data-kind=local]");
await settle();
await shot("web-welcome");
await page.click("#welcome-continue");
await until(() => window.nori.page === "home" || document.querySelector("#setup-skip, #setup-start"), 60000);
while (await page.$("#setup-skip")) await page.click("#setup-skip");
if (await page.$("#setup-start")) await page.click("#setup-start");
const profiles = JSON.parse(await ask("profiles"));
const open = profiles.list.find((p) => p.on);
report.profile = open && { kind: open.kind, label: open.label };
expect(open?.kind === "local" && open.label === "Music", "the folder's profile is open, called after it");

// Its library, once read.
await until(async () => JSON.parse(await window.ask("read", JSON.stringify({ kind: "albums", sort: "alphabeticalByName", size: 50, offset: 0 }))).albums?.length === 2, 60000).catch(() => {});
const albums = (await read({ kind: "albums", sort: "alphabeticalByName", size: 50, offset: 0 })).albums ?? [];
report.albums = albums.map((a) => `${a.name} by ${a.artist}`);
expect(albums.map((a) => a.name).join() === "First,Hits", "both albums are read");
const covers = await page.evaluate(async (list) => Promise.all(list.map(async (a) => (a.cover?.startsWith("blob:") ? (await (await fetch(a.cover)).blob()).size : 0))), albums);
report.coverBytes = covers;
expect(covers.length === 2 && covers.every((n) => n > 100), "each album's cover shows: the folder's picture and the file's");
const first = await read({ kind: "album", id: albums[0]?.id });
report.first = first.songs?.map((s) => s.title);
expect(first.songs?.map((s) => s.title).join() === "Opening,Closing", "an album's songs in their order");
const found = JSON.parse(await ask("search_typed", "closing"));
expect(JSON.stringify(found).includes("Closing"), "search finds a song by its title");

await page.evaluate(() => (location.hash = "#/albums/alphabeticalByName"));
await until(() => window.nori.page === "albums");
await until(() => [...document.querySelectorAll(".grid .card img")].filter((i) => i.complete && i.naturalWidth > 0).length === 2, 15000).catch(() => {});
await settle();
await shot("web-albums");
await page.evaluate((id) => (location.hash = `#/album/${id}`), albums[0]?.id);
await until(() => window.nori.page === "album");
await settle();
await shot("web-album");

// A song played from its file.
await ask("play", JSON.stringify(first.songs ?? []), 0, false, "", "");
await until(async () => ((await window.nori.heard?.())?.seconds ?? 0) >= 2, 30000).catch(() => {});
const heard = await page.evaluate(async () => (await window.nori.heard?.()) ?? null);
report.heardS = heard?.seconds;
expect((heard?.seconds ?? 0) >= 2, "the song plays from its file");
await ask("pause").catch(() => {});

// The add-profile dialog offers both kinds.
await page.evaluate(() => (location.hash = "#/settings/Servers"));
await until(() => document.querySelector("#add-profile-row"));
await page.click("#add-profile-row");
await until(() => document.querySelector("#add-profile [data-kind=local]"));
await settle();
await shot("web-add-profile");
await page.keyboard.press("Escape");

// Another visit: the folder kept is read again at once.
await page.goto(url, { waitUntil: "load" });
await until(() => window.nori.page === "home", 60000);
await until(async () => (await window.ask("local_waits")) == null, 15000).catch(() => {});
expect((await ask("local_waits")) == null && !(await page.$("#folder-again")), "the folder is read again without asking");
const before = (await page.evaluate(async () => (await window.nori.heard?.()) ?? null))?.seconds ?? 0;
await ask("play", JSON.stringify(first.songs ?? []), 1, false, "", "");
await until(async (b) => ((await window.nori.heard?.())?.seconds ?? 0) >= b + 2, 30000, before).catch(() => {});
const again = (await page.evaluate(async () => (await window.nori.heard?.()) ?? null))?.seconds ?? 0;
report.heardAgainS = again - before;
expect(again - before >= 2, "after a reload its songs play from their files");

report.logs = logs;
console.log(JSON.stringify(report, null, 2));
await browser.close();
fs.rmSync(music, { recursive: true, force: true });
if (failures.length) {
  console.error(`failed:\n  ${failures.join("\n  ")}`);
  process.exit(1);
}
console.log("ok");
