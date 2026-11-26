// What the shell shows over the pages: a line of news, a menu, a question and the full player.

import type { PlaylistChoice, ProfileIcon } from "./types";

/** A menu's line, or a bare glyph (`lit`: in the accent, a favorite's seal). One that switches a state (the seal)
 * `turns` into the line that switches it back, and the menu stays open. A profile's line leads with its
 * `mark` and has a second `line` under its name. */
export type Item = { label: string; icon?: string; run: () => unknown; disabled?: boolean; chosen?: boolean; lit?: boolean; turns?: () => Item; mark?: { icon: ProfileIcon; letter: string }; line?: string };
/** What the full player shows beside the cover (on a phone: in its place). */
export type Pane = "lyrics" | "queue" | "devices";
/** A menu's head: what it is about (a song: its cover, title and line), the bare glyphs beside its title, and
 * its format ("FLAC 16/44.1"), said on the line of glyphs under it. */
export type Head = { title: string; line?: string; cover?: string; glyphs?: Item[]; quality?: string };
/** Where a menu opens: beside a button's box, or at the pointer (`point`, a box of no size). */
export type Anchor = { left: number; top: number; right: number; bottom: number; point: boolean };
/** A menu: `head` what it is about, `tools` bare glyphs on a line under it, then its rows in groups. */
export type Menu = { head?: Head; tools: Item[]; groups: Item[][]; at: Anchor };

/** Beside the button `e` came from. */
export const beside = (e: MouseEvent): Anchor => {
  const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
  return { left: r.left, top: r.top, right: r.right, bottom: r.bottom, point: false };
};

/** At the pointer, for a right click. */
export const pointer = (e: MouseEvent): Anchor => ({ left: e.clientX, top: e.clientY, right: e.clientX, bottom: e.clientY, point: true });

/** The width from which the side sheet docks beside the page. */
const DOCKS = "(min-width: 1100px)";
/** How long the page takes to ease to its width beside the side sheet (the sheet's own glide). */
export const SIDE_MS = 240;

class Ui {
  toast = $state("");
  /** Moves on whenever a saved profile changes (added, renamed, removed, its icon), for what shows them. */
  profilesRev = $state(0);
  /** The menu open: on a wide window a popover beside what opened it; on a phone a menu with a head is a
   * sheet from the bottom. */
  open = $state.raw<Menu | null>(null);
  prompt = $state.raw<{ title: string; value: string; done: (v: string | null) => void } | null>(null);
  /** A question before something that cannot be undone (deleting a playlist). */
  confirming = $state.raw<{ title: string; line: string; action: string; done: (yes: boolean) => void } | null>(null);
  /** Joining someone's jam: the sheet asking for its link. */
  joining = $state(false);
  /** Adding `songIds` to a playlist: where they can go (the core's `playlist_choices`), null while read. */
  picking = $state.raw<{ songIds: string[]; choices: PlaylistChoice[] | null } | null>(null);
  /** The full player, and what it shows beside the cover (on a phone: in its place). Open, it is a step in
   * the browser's history, so Back closes it rather than leaving the page under it. */
  #full = $state(false);
  get full() {
    return this.#full;
  }
  set full(on: boolean) {
    if (on === this.#full) return;
    this.#full = on;
    if (on) history.pushState({ full: true }, "");
    else if (history.state?.full) history.back();
  }
  /** Closes the full player for a page opened from it; whether the player's step in the history is the one
   * shown, for the page to take its place. */
  leaving(): boolean {
    const step = this.#full && !!history.state?.full;
    this.#full = false;
    return step;
  }
  pane = $state<Pane | null>(null);
  /** On a wide screen, the sheet beside the pages and what it shows; closed while null. */
  side = $state<Pane | null>(null);
  /** The lyrics' source and timing, over the player. */
  lyricsInfo = $state(false);
  /** The colors a page about one cover lends the page and the chrome (`coverLook`'s), and who lent them. */
  tint = $state.raw<{ by: object; vars: string } | null>(null);
  /** The browser is wide enough for the top bar and the player bar (below it: the phone's layout). */
  wide = $state(matchMedia("(min-width: 720px)").matches);
  /** Wide enough for the side sheet to dock beside the page, which makes room for it: the page left keeps
   * 764 px, as the desktop's. Narrower, the sheet floats over the page. */
  docks = $state(matchMedia(DOCKS).matches);
  /** The side sheet was just opened or shut by a press: the page eases to its new width (a resize moves it
   * at once). */
  easing = $state(false);
  private easingTimer: ReturnType<typeof setTimeout> | undefined;
  private toastTimer: ReturnType<typeof setTimeout> | undefined;
  /** What was just taken out of the queue, while its undo is offered: what it says, and the song's id
   * (null: what a clear took). */
  undo = $state.raw<{ text: string; id: string | null } | null>(null);
  private undoTimer: ReturnType<typeof setTimeout> | undefined;

  constructor() {
    matchMedia("(min-width: 720px)").addEventListener("change", (e) => (this.wide = e.matches));
    addEventListener("popstate", () => (this.#full = !!history.state?.full));
    // Reloaded on the full player's step: nothing is open over the page it was taken on.
    if (history.state?.full) history.back();
    matchMedia(DOCKS).addEventListener("change", (e) => (this.docks = e.matches));
  }

  /** A line of news for a few seconds. */
  say(text: string) {
    this.toast = text;
    clearTimeout(this.toastTimer);
    if (text) this.toastTimer = setTimeout(() => (this.toast = ""), 4000);
  }

  /** Offers the undo of song `id` taken out of the queue (null: a clear), saying `text`, for `ms`, the
   * core's window; a later one replaces it. */
  offerUndo(text: string, id: string | null, ms: number) {
    this.undo = { text, id };
    clearTimeout(this.undoTimer);
    this.undoTimer = setTimeout(() => (this.undo = null), ms);
  }

  /** The undo offered, taken: it is offered no more. */
  takeUndo() {
    const u = this.undo;
    this.undo = null;
    clearTimeout(this.undoTimer);
    return u;
  }

  /** A menu about `head` at `at`, its rows in `groups`; one that follows another (Add to playlist) opens
   * where it was. */
  menu(head: Head, groups: Item[][], at: Anchor | undefined = this.last, tools: Item[] = []) {
    this.last = at;
    this.open = { head, tools, groups: groups.filter((g) => g.length), at: at ?? { left: innerWidth / 2, top: innerHeight / 3, right: innerWidth / 2, bottom: innerHeight / 3, point: true } };
  }

  /** Does the open menu's `it`: a line that switches a state turns in place and the menu stays open, any other
   * closes it. The line it turned into, if any. */
  choose(it: Item): Item | undefined {
    const next = it.turns?.();
    const m = this.open;
    if (next && m) {
      const swap = (items: Item[]) => items.map((x) => (x === it ? next : x));
      this.open = { ...m, head: m.head && { ...m.head, glyphs: m.head.glyphs && swap(m.head.glyphs) }, tools: swap(m.tools), groups: m.groups.map(swap) };
    } else {
      this.open = null;
    }
    void it.run();
    return next;
  }

  /** A menu of `items`, then the groups `more`, dropped from the button `e` came from. */
  drop(e: MouseEvent, items: Item[], more: Item[][] = []) {
    this.last = beside(e);
    this.open = { tools: [], groups: [items, ...more], at: this.last };
  }

  /** Where the last menu opened. */
  private last: Anchor | undefined;

  /** Asks for a line of text (a playlist's name); null when cancelled. */
  ask(title: string, value = ""): Promise<string | null> {
    return new Promise((done) => {
      this.prompt = {
        title,
        value,
        done: (v) => {
          this.prompt = null;
          done(v && v.trim() ? v.trim() : null);
        },
      };
    });
  }

  /** Asks whether to `action` (the button's word), `title` and `line` saying what goes; true for yes. */
  confirm(title: string, line: string, action: string): Promise<boolean> {
    return new Promise((done) => {
      this.confirming = {
        title,
        line,
        action,
        done: (yes) => {
          this.confirming = null;
          done(yes);
        },
      };
    });
  }

  /** The page changed: no menu or dialog opened over the last one stays open over this one. */
  closeAll() {
    this.open = null;
    this.prompt?.done(null);
    this.confirming?.done(false);
    this.joining = false;
    this.picking = null;
  }

  /** The side sheet stands docked beside the page, which is narrower by its room (`--dock`). */
  get docked() {
    return this.wide && this.docks && !!this.side;
  }

  /** Shows `pane`: on a wide screen in the full player when it is open, else in the side sheet (closed
   * by asking for it again); on a phone in the full player, and asked again it goes back to the cover. */
  show(pane: Pane) {
    if (this.wide && this.full) this.pane = pane;
    else if (this.wide) {
      const side = this.side === pane ? null : pane;
      if (!side !== !this.side && this.docks) {
        this.easing = true;
        clearTimeout(this.easingTimer);
        this.easingTimer = setTimeout(() => (this.easing = false), SIDE_MS);
      }
      this.side = side;
    } else {
      this.full = true;
      this.pane = this.pane === pane ? null : pane;
    }
  }
}

export const ui = new Ui();
