// What the shell shows over the pages: a line of news, a menu, a question and the full player.

export type Item = { label: string; icon?: string; run: () => unknown; disabled?: boolean; chosen?: boolean };
/** What the full player shows beside the cover (on a phone: in its place). */
export type Pane = "lyrics" | "queue" | "devices";
/** A menu's head: what it is about (a song: its cover, title and line). */
export type Head = { title: string; line?: string; cover?: string };
/** Where a menu opens: beside a button's box, or at the pointer (`point`, a box of no size). */
export type Anchor = { left: number; top: number; right: number; bottom: number; point: boolean };
/** A menu: `head` what it is about, `more` folded under More. */
export type Menu = { head?: Head; items: Item[]; more: Item[]; at: Anchor };

/** Beside the button `e` came from. */
export const beside = (e: MouseEvent): Anchor => {
  const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
  return { left: r.left, top: r.top, right: r.right, bottom: r.bottom, point: false };
};

/** At the pointer, for a right click. */
export const pointer = (e: MouseEvent): Anchor => ({ left: e.clientX, top: e.clientY, right: e.clientX, bottom: e.clientY, point: true });

class Ui {
  toast = $state("");
  /** The menu open: on a wide window a popover beside what opened it; on a phone a menu with a head is a
   * sheet from the bottom. */
  open = $state.raw<Menu | null>(null);
  prompt = $state.raw<{ title: string; value: string; done: (v: string | null) => void } | null>(null);
  /** A question before something that cannot be undone (deleting a playlist). */
  confirming = $state.raw<{ title: string; line: string; action: string; done: (yes: boolean) => void } | null>(null);
  /** Joining someone's jam: the sheet asking for its link. */
  joining = $state(false);
  /** The playlists `songIds` may be added to, with a new one named there. */
  picking = $state.raw<{ songIds: string[]; playlists: { id: string; name: string }[] } | null>(null);
  /** The full player, and what it shows beside the cover (on a phone: in its place). */
  full = $state(false);
  pane = $state<Pane | null>(null);
  /** On a wide screen, the sheet beside the pages and what it shows; closed while null. */
  side = $state<Pane | null>(null);
  /** The lyrics' source and timing, over the player. */
  lyricsInfo = $state(false);
  /** The colors a page about one cover lends the page and the chrome (`coverLook`'s), and who lent them. */
  tint = $state.raw<{ by: object; vars: string } | null>(null);
  /** The browser is wide enough for the top bar and the player bar (below it: the phone's layout). */
  wide = $state(matchMedia("(min-width: 720px)").matches);
  private toastTimer: ReturnType<typeof setTimeout> | undefined;

  constructor() {
    matchMedia("(min-width: 720px)").addEventListener("change", (e) => (this.wide = e.matches));
  }

  /** A line of news for a few seconds. */
  say(text: string) {
    this.toast = text;
    clearTimeout(this.toastTimer);
    if (text) this.toastTimer = setTimeout(() => (this.toast = ""), 4000);
  }

  /** A menu about `head` at `at`; one that follows another (Add to playlist…) opens where it was. */
  menu(head: Head, items: Item[], more: Item[] = [], at: Anchor | undefined = this.last) {
    this.last = at;
    this.open = { head, items, more, at: at ?? { left: innerWidth / 2, top: innerHeight / 3, right: innerWidth / 2, bottom: innerHeight / 3, point: true } };
  }

  /** A menu of `items` dropped from the button `e` came from. */
  drop(e: MouseEvent, items: Item[]) {
    this.last = beside(e);
    this.open = { items, more: [], at: this.last };
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

  /** Shows `pane`: on a wide screen in the full player when it is open, else in the side sheet (closed
   * by asking for it again); on a phone in the full player, and asked again it goes back to the cover. */
  show(pane: Pane) {
    if (this.wide && this.full) this.pane = pane;
    else if (this.wide) this.side = this.side === pane ? null : pane;
    else {
      this.full = true;
      this.pane = this.pane === pane ? null : pane;
    }
  }
}

export const ui = new Ui();
