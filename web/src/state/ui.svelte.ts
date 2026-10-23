// What the shell shows over the pages: a line of news, a song's sheet, a menu dropped from a button, a
// question, the devices and the full player.

export type Item = { label: string; icon?: string; run: () => unknown; disabled?: boolean; chosen?: boolean };
/** What the full player shows in its cover's place. */
export type Pane = "lyrics" | "queue";
/** A sheet's head: what it is about (a song: its cover, title and line). */
export type Head = { title: string; line?: string; cover?: string };

class Ui {
  toast = $state("");
  /** A sheet of grouped rows (a song's menu); `more` folded under More. */
  sheet = $state.raw<{ head: Head; items: Item[]; more: Item[] } | null>(null);
  /** A menu dropped from `at` (the button that opened it). */
  popover = $state.raw<{ at: DOMRect; items: Item[] } | null>(null);
  prompt = $state.raw<{ title: string; value: string; done: (v: string | null) => void } | null>(null);
  /** The full player, and what it shows in its cover's place. */
  full = $state(false);
  pane = $state<Pane | null>(null);
  /** The devices and the jam, as a sheet. */
  devices = $state(false);
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

  menu(head: Head, items: Item[], more: Item[] = []) {
    this.sheet = { head, items, more };
  }

  /** A menu of `items` dropped from the button `e` came from. */
  drop(e: MouseEvent, items: Item[]) {
    this.popover = { at: (e.currentTarget as HTMLElement).getBoundingClientRect(), items };
  }

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

  /** Opens the full player showing `pane`; again, it goes back to the cover. */
  show(pane: Pane) {
    this.pane = this.full && this.pane === pane ? null : pane;
    this.full = true;
  }
}

export const ui = new Ui();
