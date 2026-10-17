// The page: signs in, routes between the pages and keeps the player bar. The core and the engine live
// in the worker (`rpc.ts`); the sound card is an AudioWorklet the worker feeds directly.

import { ask, boot, facts, onMovedAway, playerSoon, takeOver } from "./rpc.js";
import { h, icon } from "./dom.js";
import { setup } from "./player.js";
import * as views from "./views.js";
import { W } from "./words.js";

const page = () => document.getElementById("page")!;
const app = () => document.getElementById("app")!;

/** The pages by their address. */
async function route(): Promise<HTMLElement> {
  const [, top, a = ""] = location.hash.replace(/^#\/?/, "/").split("/");
  const arg = decodeURIComponent(a);
  switch (top) {
    case "library":
      return views.library();
    case "albums":
      return views.albums(arg || "alphabeticalByName");
    case "album":
      return views.album(arg);
    case "artists":
      return views.artists();
    case "artist":
      return views.artist(arg);
    case "playlists":
      return views.playlists();
    case "playlist":
      return views.playlist(arg);
    case "genres":
      return views.genres();
    case "genre":
      return views.genre(arg);
    case "songs":
      return views.songs();
    case "favorites":
      return views.favorites();
    case "search":
      return views.search(arg);
    case "settings":
      return views.settings(signOut);
    default:
      return views.home();
  }
}

let shown = 0;

async function show() {
  const n = ++shown;
  const top = location.hash.replace(/^#\/?/, "").split("/")[0] || "home";
  document.querySelectorAll("#nav a").forEach((a) => a.classList.toggle("on", a.getAttribute("data-top") === top || (top !== "home" && top !== "search" && top !== "settings" && a.getAttribute("data-top") === "library")));
  if (!page().firstChild) page().replaceChildren(h("p", { class: "loading" }, W.loading));
  try {
    const el = await route();
    if (n !== shown) return;
    page().replaceChildren(el);
    window.scrollTo(0, 0);
    facts.page = top;
    delete facts.pageError;
  } catch (e) {
    if (n !== shown) return;
    const why = String((e as Error).message ?? e);
    page().replaceChildren(h("p", { class: "empty" }, W.failed(why)));
    facts.page = top;
    facts.pageError = why;
  }
}

function nav(): HTMLElement {
  const item = (top: string, href: string, i: string, label: string) => h("a", { href, "data-top": top }, icon(i), h("span", null, label));
  return h("nav", { id: "nav" }, h("div", { class: "brand" }, W.appName), item("home", "#/", "home", W.home), item("library", "#/library", "library", W.library), item("search", "#/search", "search", W.search), item("settings", "#/settings", "settings", W.settings));
}

function login(failed?: string): HTMLElement {
  const form = h(
    "form",
    { id: "login", class: "login" },
    h("h1", null, W.appName),
    h("p", { class: "sub" }, W.signInLead),
    h("input", { name: "url", value: location.origin, "aria-label": W.server, placeholder: W.server, autocomplete: "url" }),
    h("input", { name: "user", value: "admin", "aria-label": W.user, placeholder: W.user, autocomplete: "username" }),
    h("input", { name: "password", type: "password", value: "admin", "aria-label": W.password, placeholder: W.password, autocomplete: "current-password" }),
    h("button", { class: "pill wide" }, W.signIn),
    failed ? h("p", { class: "error" }, W.signInFailed(failed)) : null,
  );
  form.onsubmit = async (e) => {
    e.preventDefault();
    const f = new FormData(form);
    const t0 = performance.now();
    form.querySelector("button")!.textContent = W.signingIn;
    try {
      await ask("login", String(f.get("url")).replace(/\/$/, ""), f.get("user"), f.get("password"));
      facts.loginMs = Math.round(performance.now() - t0);
      enter();
    } catch (err) {
      app().replaceChildren(login(String((err as Error).message ?? err)));
    }
  };
  return form;
}

function enter() {
  app().replaceChildren(nav(), h("main", { id: "page" }), h("div", { id: "mini", class: "mini", hidden: true }), h("div", { id: "sheet", class: "sheet", hidden: true }), h("div", { id: "toast", class: "toast", hidden: true }));
  setup();
  window.onhashchange = () => void show();
  // The player module once the first page shows, so Play never waits for it.
  void show().then(playerSoon);
}

async function signOut() {
  await ask("logout");
  location.hash = "";
  location.reload();
}

/** Another tab has nori: this one offers to take it over. */
function elsewhere(): HTMLElement {
  return h(
    "div",
    { class: "login" },
    h("h1", null, W.elsewhere),
    h("p", { class: "sub" }, W.elsewhereLead),
    h("button", { class: "pill wide", id: "use-here", on: { click: () => void takeOver() } }, W.useHere),
  );
}

/** A new build of the client waiting in the service worker takes over now, and the page loads again:
 * true then. Otherwise the service worker is registered (or checked for a new build). */
async function updated(): Promise<boolean> {
  if (!("serviceWorker" in navigator)) return false;
  const reg = await navigator.serviceWorker.getRegistration();
  if (reg?.waiting && navigator.serviceWorker.controller) {
    const taken = new Promise((r) => navigator.serviceWorker.addEventListener("controllerchange", r, { once: true }));
    reg.waiting.postMessage("take over");
    await taken;
    location.reload();
    return true;
  }
  void navigator.serviceWorker.register("sw.js").then(() => (facts.serviceWorker = true));
  return false;
}

async function start() {
  if (await updated()) return;
  onMovedAway(() => {
    facts.movedAway = true;
    app().replaceChildren(elsewhere());
  });
  if (!(await boot())) {
    facts.elsewhere = true;
    app().replaceChildren(elsewhere());
    return;
  }
  if (await ask<boolean>("resume")) enter();
  else app().replaceChildren(login());
}

void start();
(window as unknown as { ask: unknown }).ask = ask;
