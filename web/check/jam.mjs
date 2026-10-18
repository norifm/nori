// A jam between two browsers (two headless Chrome profiles, each owning its own databases) over a
// server with the relay: octo-fiesta in front of the local Navidrome (tools/jam-server.sh, port 5274)
// through web/serve.py:
//
//   python3 web/serve.py --port 8081 --server http://localhost:5274
//   node web/check/jam.mjs http://localhost:8081/
//
// The host starts a jam and plays; the guest joins with the invite link and sees the host's song; the
// guest's pick arrives as a request, the host accepts it, and it lands in the host's queue; the guest
// listens along and hears music; the host ends the jam and the guest is back on its own profile.
import puppeteer from "puppeteer-core";

const url = process.argv[2] ?? "http://localhost:8081/";
const browser = await puppeteer.launch({
  executablePath: process.env.CHROME ?? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  headless: "new",
  args: ["--autoplay-policy=no-user-gesture-required", "--mute-audio"],
});
const logs = [];
const failures = [];
const report = {};
const expect = (ok, what) => ok || failures.push(what);

/** A page in a fresh browser profile, signed in and on Listen Now. */
async function open(name) {
  const ctx = await browser.createBrowserContext();
  const page = await ctx.newPage();
  await page.setViewport({ width: 1280, height: 800 });
  page.on("pageerror", (e) => logs.push(`${name} pageerror: ${e}`));
  page.on("console", (m) => m.type() === "error" && !m.text().includes("404") && logs.push(`${name} console: ${m.text()}`));
  await page.goto(url, { waitUntil: "load" });
  await page.waitForFunction(() => window.nori.coreMs !== undefined, { timeout: 60000 });
  if (await page.$("#login")) await page.click("#login button");
  await page.waitForFunction(() => window.nori.page === "home" && window.nori.playerReadyAt !== undefined, { timeout: 60000 });
  return page;
}

const ask = (page, call, ...args) => page.evaluate((c, a) => window.ask(c, ...a), call, args);
const json = async (page, call, ...args) => JSON.parse(await ask(page, call, ...args));
const until = (page, f, arg, timeout = 30000) => page.waitForFunction(f, { timeout, polling: 250 }, arg);

const host = await open("host");
const guest = await open("guest");

// The host plays an album with a click (the sound card needs it), then starts the jam.
const album = await host.evaluate(async () => JSON.parse(await window.ask("read", JSON.stringify({ kind: "albums", sort: "alphabeticalByName", size: 50, offset: 0 }))).albums.find((a) => a.name === "Second Wind").id);
// A click starts the sound only in the tab in front.
await host.bringToFront();
await host.evaluate((id) => (location.hash = `#/album/${id}`), album);
await host.waitForSelector(".songs li");
await (await host.$$(".songs li"))[0].click();
await until(host, () => (window.nori.heard?.seconds ?? 0) > 1).catch(async (e) => {
  console.log("host", JSON.stringify(await host.evaluate(() => window.nori)), await ask(host, "now"), logs.join("\n"));
  throw e;
});
const link = await ask(host, "jam_open");
await until(host, async () => JSON.parse(await window.ask("jam")).hosting);
report.host = { link: !!link, jam: await json(host, "jam") };
expect(typeof link === "string" && link.length > 0, "the host's jam has an invite link");

// The guest joins with the link from its Jam panel.
await guest.bringToFront();
await guest.evaluate(() => (location.hash = "#/settings"));
await guest.waitForSelector(".jam .link");
const fields = await guest.$$(".jam .link");
await fields[0].type(link);
await fields[1].type("Guest Browser");
const buttons = await guest.$$(".jam .pill");
await buttons[buttons.length - 1].click();
await until(guest, async () => JSON.parse(await window.ask("jam"))?.guest === true, null, 60000);
await until(guest, async () => JSON.parse(await window.ask("now"))?.song?.title, null, 60000);
const seen = await json(guest, "now");
const hostNow = await json(host, "now");
report.guest = { jam: await json(guest, "jam"), song: seen.song?.title, hostSong: hostNow.song?.title, jamFlag: seen.jam };
expect(seen.jam && seen.song?.title === hostNow.song?.title, "the guest sees the host's song");
await until(host, async () => JSON.parse(await window.ask("jam")).members.length >= 2);
report.members = (await json(host, "jam")).members.map((m) => `${m.name}:${m.role}`);

// The guest picks a song: a request the host accepts, which lands in the host's queue.
const pick = await guest.evaluate(async () => {
  const albums = JSON.parse(await window.ask("read", JSON.stringify({ kind: "albums", sort: "alphabeticalByName", size: 50, offset: 0 }))).albums;
  const songs = JSON.parse(await window.ask("read", JSON.stringify({ kind: "album", id: albums.find((a) => a.name === "Blue Hour").id }))).songs;
  return songs[0];
});
await ask(guest, "enqueue", JSON.stringify([pick]), true);
await until(host, async () => JSON.parse(await window.ask("jam")).pending.length > 0);
const request = (await json(host, "jam")).pending[0];
await ask(host, "jam_decide", request.request, true);
await until(host, async (id) => JSON.parse(await window.ask("now")).queue.some((s) => s.id === id), pick.id);
report.request = { title: request.title, from: request.from, accepted: true };

// The guest listens along: its own player plays the jam's music.
await ask(host, "jam_along", true);
await until(guest, async () => JSON.parse(await window.ask("jam")).along);
await guest.evaluate(() => (location.hash = "#/"));
await guest.waitForSelector("#mini:not([hidden])", { timeout: 30000 });
await guest.click("#mini");
await guest.waitForSelector(".tab");
await guest.evaluate(() => document.querySelectorAll(".tab")[2].click());
await guest.waitForSelector(".jam .switch");
await guest.evaluate(() => document.querySelector(".jam .switch").click());
await until(guest, async () => JSON.parse(await window.ask("jam")).listening === "Playing", null, 30000).catch(() => {});
await until(guest, () => (window.nori.heard?.seconds ?? 0) > 2, null, 30000).catch(() => {});
const heard = await guest.evaluate(() => window.nori.heard);
report.along = { listening: (await json(guest, "jam")).listening, heardS: heard?.seconds ?? 0, dry: heard?.dry ?? null };
expect((heard?.seconds ?? 0) > 2, "a guest listening along hears the jam");

// The host ends the jam; the guest leaves its guest profile.
await ask(host, "jam_close");
await until(guest, async () => JSON.parse(await window.ask("jam"))?.guest !== true, null, 60000);
report.ended = { guestProfile: await json(guest, "profile") };
expect(!(await json(guest, "jam"))?.guest, "the guest leaves when the host ends the jam");

report.failures = failures;
console.log(JSON.stringify(report, null, 1));
if (logs.length) console.log(logs.slice(0, 30).join("\n"));
await browser.close();
process.exit(failures.length ? 1 : 0);
