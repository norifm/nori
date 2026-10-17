// What the shell shows over the pages: a line of news, an action sheet, a question, the side panel and
// the full player.

export type Item = { label: string; icon?: string; run: () => unknown; danger?: boolean };
/** What the side panel (or the full player's) shows. */
export type Pane = "lyrics" | "queue" | "devices" | "jam";

class Ui {
  toast = $state("");
  sheet = $state.raw<{ title: string; items: Item[] } | null>(null);
  prompt = $state.raw<{ title: string; value: string; done: (v: string | null) => void } | null>(null);
  /** The panel beside the page on a wide screen. */
  panel = $state<Pane | null>(null);
  /** The full player, and what it shows beside the cover (on a phone: in its place). */
  full = $state(false);
  pane = $state<Pane | null>(null);
  /** The lyrics' source and timing, over the player. */
  lyricsInfo = $state(false);
  /** The browser is wide enough for the sidebar and the panel. */
  wide = $state(matchMedia("(min-width: 900px)").matches);
  private toastTimer: ReturnType<typeof setTimeout> | undefined;

  constructor() {
    matchMedia("(min-width: 900px)").addEventListener("change", (e) => (this.wide = e.matches));
  }

  /** A line of news for a few seconds. */
  say(text: string) {
    this.toast = text;
    clearTimeout(this.toastTimer);
    if (text) this.toastTimer = setTimeout(() => (this.toast = ""), 4000);
  }

  menu(title: string, items: Item[]) {
    this.sheet = { title, items };
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

  /** Shows `pane`: in the panel on a wide screen, else in the full player; again, hides it. */
  show(pane: Pane) {
    if (this.wide && !this.full) this.panel = this.panel === pane ? null : pane;
    else {
      this.full = true;
      this.pane = this.pane === pane ? null : pane;
    }
  }
}

export const ui = new Ui();
