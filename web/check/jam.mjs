// Two browsers (two headless Chrome profiles, each owning its own databases, each a device of the
// account) over a server with the relay: octo-fiesta in front of the local Navidrome
// (tools/jam-server.sh, port 5274) through web/serve.py:
//
//   python3 web/serve.py --port 8081 --server http://localhost:5274
//   node web/check/jam.mjs http://localhost:8081/ [playon,jam]
//
// Play On: the first browser plays and moves the music to the second from its devices pane; the second
// plays it on from there and the first shows it playing there; then it is moved back.
// A jam: the host starts one and plays; the guest joins with the invite link and sees the host's song;
// the guest's pick arrives as a request, the host accepts it, and it lands in the host's queue; the
// guest listens along and hears music; the host ends the jam and the guest is back on its own profile.
import puppeteer from "puppeteer-core";

const url = process.argv[2] ?? "http://localhost:8081/";
const only = process.argv[3]?.split(",");
const browser = await puppeteer.launch({
  executablePath: process.env.CHROME ?? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  headless: "new",
  args: ["--autoplay-policy=no-user-gesture-required", "--mute-audio", "--disable-background-timer-throttling", "--disable-renderer-backgrounding", "--disable-backgrounding-occluded-windows"],
});
const logs = [];
const failures = [];
const report = {};
const expect = (ok, what) => ok || failures.push(what);

/** A page in a fresh browser profile, signed in and on Home. */
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
const heard = (page) => page.evaluate(async () => (await window.nori.heard?.())?.seconds ?? 0);

/** Opens pane `name` beside the page from the player bar. */
async function pane(page, name) {
  if (!(await page.$(`#full #${name}`))) await page.click(`#bar [data-pane=${name}]`);
  await page.waitForSelector(`#full #${name}`);
}

const host = await open("host");
const guest = await open("guest");

// The host plays an album with a click (the sound card needs it).
const album = await host.evaluate(async () => JSON.parse(await window.ask("read", JSON.stringify({ kind: "albums", sort: "alphabeticalByName", size: 50, offset: 0 }))).albums.find((a) => a.name === "Second Wind").id);
// A click starts the sound only in the tab in front.
await host.bringToFront();
await host.evaluate((id) => (location.hash = `#/album/${id}`), album);
await host.waitForSelector(".songs li .play");
await (await host.$$(".songs li .play"))[0].click();
await until(host, async () => ((await window.nori.heard?.())?.seconds ?? 0) > 1).catch(async (e) => {
  console.log("host playback", await host.evaluate(async () => {
    const n = JSON.parse(await window.ask("now"));
    return { heard: await window.nori.heard?.(), state: n.state, positionMs: n.positionMs, song: n.song?.title };
  }), logs.join("\n"));
  throw e;
});

if (!only || only.includes("playon")) {
  // A browser opened while another device plays shows that playback, and its Pause reaches it.
  const late = await open("late");
  await until(late, async () => !!JSON.parse(await window.ask("now"))?.device, null, 30000).catch(() => {});
  const follows = (await json(late, "now")).device;
  await ask(late, "toggle");
  await until(host, async () => JSON.parse(await window.ask("now"))?.state === "Paused", null, 15000).catch(() => {});
  const paused = (await json(host, "now")).state;
  await ask(late, "toggle");
  await until(host, async () => JSON.parse(await window.ask("now"))?.state === "Playing", null, 15000).catch(() => {});
  report.lateBrowser = { follows, hostAfterPause: paused, hostAfterPlay: (await json(host, "now")).state };
  expect(!!follows && paused === "Paused" && report.lateBrowser.hostAfterPlay === "Playing", "a browser opened later shows the playing device and pauses and plays it");
  await late.browserContext().close();
  // The other browser shows in the devices pane once the relay has it; a click moves the music there.
  await pane(host, "devices");
  // The other browser by its id (the relay may still list browsers of earlier runs).
  const there = (await json(guest, "devices", "{}")).me;
  await host.waitForSelector(`#devices [data-device="${there}"]`, { timeout: 60000 });
  const song = (await json(host, "now")).song.title;
  const guestHeard = await heard(guest);
  const t0 = Date.now();
  await host.click(`#devices [data-device="${there}"]`);
  await until(guest, async () => JSON.parse(await window.ask("now"))?.state === "Playing", null, 30000).catch(() => {});
  await until(guest, async (h) => ((await window.nori.heard?.())?.seconds ?? 0) > h + 2, guestHeard, 30000).catch(() => {});
  const movedMs = Date.now() - t0;
  const onGuest = await json(guest, "now");
  await until(host, async () => !!JSON.parse(await window.ask("now"))?.device, null, 15000).catch(() => {});
  const onHost = await json(host, "now");
  const shownThere = await host.$eval("#bar", (b) => b.textContent);
  const hostHeard = await heard(host);
  await new Promise((r) => setTimeout(r, 1500));
  const hostQuiet = (await heard(host)) - hostHeard;
  report.playOn = { song, there: onGuest.song?.title, guestState: onGuest.state, guestHeardS: (await heard(guest)) - guestHeard, hostShows: onHost.device, hostHeardWhileThere: hostQuiet, movedMs };
  expect(onGuest.song?.title === song && onGuest.state === "Playing" && !onGuest.device && report.playOn.guestHeardS > 2, "music moved to another browser plays there");
  expect(onHost.device && shownThere.includes(onHost.device) && hostQuiet < 0.2, "the browser it moved from shows it playing there and plays nothing");
  // Back here.
  const back = await heard(host);
  await host.click('#devices [data-device=""]');
  await until(host, async () => { const n = JSON.parse(await window.ask("now")); return n.state === "Playing" && !n.device; }, null, 30000).catch(() => {});
  await until(host, async (h) => ((await window.nori.heard?.())?.seconds ?? 0) > h + 2, back, 30000).catch(() => {});
  const home = await json(host, "now");
  // The other browser shows it playing here, and plays nothing itself.
  await until(guest, async () => !!JSON.parse(await window.ask("now"))?.device, null, 15000).catch(() => {});
  const left = await json(guest, "now");
  const guestBefore = await heard(guest);
  await new Promise((r) => setTimeout(r, 1500));
  report.playOnBack = { song: home.song?.title, state: home.state, device: home.device, heardS: (await heard(host)) - back, guestShows: left.device, guestHeardS: (await heard(guest)) - guestBefore };
  expect(home.song?.title === song && home.state === "Playing" && !home.device && report.playOnBack.heardS > 2, "music moved back plays here again");
  expect(!!left.device && report.playOnBack.guestHeardS < 0.2, "the other browser shows it playing here and plays nothing");
}

if (!only || only.includes("jam")) {
  const link = await ask(host, "jam_open");
  await until(host, async () => JSON.parse(await window.ask("jam")).hosting);
  report.host = { link: !!link, jam: await json(host, "jam") };
  expect(typeof link === "string" && link.length > 0, "the host's jam has an invite link");

  // The guest joins with the link from its devices pane.
  await guest.bringToFront();
  await pane(guest, "devices");
  await guest.waitForSelector("#jam-link");
  await guest.type("#jam-link", link);
  await guest.type("#jam-name", "Guest Browser");
  await guest.click("#jam-join");
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
  // The host sees it over its queue and accepts it there.
  await host.bringToFront();
  await pane(host, "queue");
  await host.waitForSelector("#jam .request .solid");
  const request = (await json(host, "jam")).pending[0];
  await host.click("#jam .request .solid");
  await until(host, async (id) => JSON.parse(await window.ask("now")).queue.some((s) => s.id === id), pick.id);
  report.request = { title: request.song.title, from: request.from, accepted: true };

  // The guest listens along: its own player plays the jam's music.
  await ask(host, "jam_along", true);
  await until(guest, async () => JSON.parse(await window.ask("jam")).along);
  await guest.bringToFront();
  await guest.evaluate(() => (location.hash = "#/"));
  await pane(guest, "queue");
  await guest.waitForSelector("#jam-listen", { timeout: 30000 });
  await guest.click("#jam-listen");
  await until(guest, async () => JSON.parse(await window.ask("jam")).listening === "Playing", null, 30000).catch(() => {});
  await until(guest, async () => ((await window.nori.heard?.())?.seconds ?? 0) > 2, null, 30000).catch(() => {});
  const along = await guest.evaluate(() => window.nori.heard?.());
  report.along = { listening: (await json(guest, "jam")).listening, heardS: along?.seconds ?? 0, dry: along?.dry ?? null };
  expect((along?.seconds ?? 0) > 2, "a guest listening along hears the jam");

  // The host ends the jam; the guest leaves its guest profile.
  await ask(host, "jam_close");
  await until(guest, async () => JSON.parse(await window.ask("jam"))?.guest !== true, null, 60000);
  report.ended = { guestProfile: await json(guest, "profile") };
  expect(!(await json(guest, "jam"))?.guest, "the guest leaves when the host ends the jam");
}

report.failures = failures;
console.log(JSON.stringify(report, (key, value) => key === "link" ? !!value : value, 1));
if (logs.length) console.log(logs.slice(0, 30).join("\n"));
await browser.close();
process.exit(failures.length ? 1 : 0);
