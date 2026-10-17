// Jams, as Spotify and the Android app have them: start one and share its link, or join someone's with
// theirs; the host decides what guests ask for. The rules are the core's; this draws them and asks.

import { ask, facts, onSaid, player, playerUp } from "./rpc.js";
import { h, icon } from "./dom.js";
import { soundCard } from "./player.js";
import { W } from "./words.js";

export type Jam = {
  start: string;
  guest: boolean;
  hosting: boolean;
  link: string | null;
  host: string | null;
  you: string | null;
  members: { id: string; name: string; role: string }[];
  pending: { request: number; from: string; title: string; artist: string }[];
  along: boolean;
  listening: string | null;
  asks: boolean;
};

/** The jam now; null before the player runs. */
export async function jam(): Promise<Jam | null> {
  if (!player) return null;
  const j = JSON.parse(await ask<string>("jam")) as Jam | null;
  facts.jam = j && { start: j.start, hosting: j.hosting, guest: j.guest, members: j.members.length, pending: j.pending.length, listening: j.listening, link: j.link };
  return j;
}

/** Joins the jam `link` invites to; the page starts again over the guest's profile. */
export async function join(link: string, name: string): Promise<string> {
  const said = await ask<string>("jam_join", link.trim(), name.trim() || W.jamGuestName);
  if (said === "joined") afterSwitch();
  return said;
}

/** The library and the player are the other profile's now: the pages are drawn again. */
function afterSwitch() {
  location.hash = "#/";
  window.dispatchEvent(new HashChangeEvent("hashchange"));
  window.dispatchEvent(new Event("nori-profile"));
}

/** The jam's panel: start or join; the invite, members and requests while hosting; the host and
 * listening along as a guest. `redraw` draws it again after a change. */
export async function panel(redraw: () => void): Promise<HTMLElement> {
  const j = await jam();
  const box = h("div", { class: "jam" });
  const act = (f: () => Promise<unknown>) => async () => {
    try {
      await f();
    } catch (e) {
      box.append(h("p", { class: "error" }, W.failed(String((e as Error).message ?? e))));
      return;
    }
    redraw();
  };
  if (!j) {
    // The player module runs the remote: it starts on the first Play.
    box.append(h("p", { class: "sub" }, W.jamLead), h("button", { class: "pill", on: { click: act(playerUp) } }, icon("jam"), W.jamStart));
    return box;
  }
  if (j.guest) {
    const listening = j.listening === "Playing";
    box.append(
      h("h3", null, W.jamOf(j.host ?? "")),
      h("p", { class: "sub" }, j.asks ? W.jamAsks : W.jamAdds),
      h("label", { class: "setting" }, h("span", { class: "title" }, W.jamListen), h("input", { type: "checkbox", class: "switch", checked: listening, on: { change: (e) => void act(async () => {
        const on = (e.target as HTMLInputElement).checked;
        // Listening along plays here: the sound card is made in this click.
        if (on) await soundCard();
        await ask("jam_listen", on);
      })() } })),
      j.listening === "HostOff" ? h("p", { class: "sub" }, W.jamHostOff) : "",
      members(j),
      h("button", { class: "pill", on: { click: act(async () => (await ask<boolean>("jam_leave")) ? afterSwitch() : location.reload()) } }, W.jamLeave),
    );
    return box;
  }
  if (j.hosting) {
    const link = j.link ?? "";
    box.append(
      h("h3", null, W.jamYours),
      h(
        "div",
        { class: "invite" },
        h("input", { class: "link", readonly: true, value: link, "aria-label": W.jamLink }),
        h("button", { class: "pill", on: { click: () => void navigator.clipboard?.writeText(link).then(() => (copied.textContent = W.copied)) } }, W.copy),
      ),
      h("label", { class: "setting" }, h("span", { class: "title" }, W.jamAlong), h("input", { type: "checkbox", class: "switch", checked: j.along, on: { change: (e) => void act(() => ask("jam_along", (e.target as HTMLInputElement).checked))() } })),
      j.pending.length ? h("h4", null, W.jamRequests) : "",
      ...j.pending.map((p) =>
        h(
          "div",
          { class: "request" },
          h("div", { class: "row-text" }, h("span", { class: "title" }, p.title), h("span", { class: "sub" }, `${p.artist} · ${W.jamAskedBy(p.from)}`)),
          h("button", { class: "pill small", on: { click: act(() => ask("jam_decide", p.request, true)) } }, W.accept),
          h("button", { class: "pill small", on: { click: act(() => ask("jam_decide", p.request, false)) } }, W.decline),
        ),
      ),
      members(j),
      h("button", { class: "pill", on: { click: act(() => ask("jam_close")) } }, W.jamEnd),
    );
    const copied = h("span", { class: "sub" });
    box.append(copied);
    return box;
  }
  const field = h("input", { class: "link", placeholder: W.jamPaste, "aria-label": W.jamLink }) as HTMLInputElement;
  const name = h("input", { class: "link", placeholder: W.jamName, "aria-label": W.jamName }) as HTMLInputElement;
  box.append(
    h("p", { class: "sub" }, W.jamLead),
    j.start === "Unavailable" ? h("p", { class: "sub" }, W.jamUnavailable) : h("button", { class: "pill", disabled: j.start === "Starting", on: { click: act(() => ask("jam_open")) } }, icon("jam"), j.start === "Starting" ? W.jamStarting : W.jamStart),
    h("h4", null, W.jamJoin),
    field,
    name,
    h(
      "button",
      {
        class: "pill",
        on: {
          click: act(async () => {
            const said = await join(field.value, name.value);
            if (said !== "joined") box.append(h("p", { class: "sub" }, said === "own" ? W.jamOwn : W.jamEnded));
          }),
        },
      },
      W.jamJoin,
    ),
  );
  return box;
}

function members(j: Jam): HTMLElement {
  return h("ul", { class: "members" }, ...j.members.map((m) => h("li", null, icon(m.role === "Host" ? "jam" : "artist"), h("span", null, m.name), h("span", { class: "sub" }, W.roles[m.role] ?? m.role))));
}

/** A jam this guest was in ended: its profile goes, the one before opens again. */
onSaid((s) => {
  if (s.kind !== "jamEnded") return;
  void ask<boolean>("jam_leave").then((back) => (back ? afterSwitch() : location.reload()));
});
