// The page shown, by its address (`#/album/<id>`): read from the hash whenever it changes.

/** Pages under the Library tab on a phone. */
const LIBRARY = ["library", "albums", "artists", "playlists", "genres", "songs", "favorites"];
/** Pages opened from a tab's page, each with the one it sits under when its address is opened alone. */
const OPENED: Record<string, string> = { album: "albums", artist: "artists", playlist: "playlists", genre: "genres", mix: "home" };

/** `a` unescaped; as typed when it holds a malformed escape. */
function decoded(a: string): string {
  try {
    return decodeURIComponent(a);
  } catch {
    return a;
  }
}

class Route {
  top = $state("home");
  arg = $state("");
  /** A page under `arg` (a plugin's, under Settings › Plugins). */
  sub = $state("");
  /** A page under `sub` (one of that plugin's own pages, by its page row). */
  page = $state("");
  /** Bumped by every visit, the same address again too: a page reads itself again. */
  visit = $state(0);

  /** The tab's page the page shown was opened from (an album opened from Home: "home"); a tab's page is its
   * own. Kept in the history entry, so Back and Forward find it again. */
  from = $state("");
  /** The tab the page sits under on a phone. */
  tab = $derived(LIBRARY.includes(this.from) ? "library" : this.from === "row" ? "home" : this.from === "equalizer" || this.from === "support" ? "settings" : this.from);

  read() {
    const [, top = "", a = "", b = "", c = ""] = location.hash.replace(/^#\/?/, "/").split("/");
    this.top = top || "home";
    const kept = (history.state as { from?: string } | null)?.from;
    this.from = !(this.top in OPENED) ? this.top : (kept ?? (this.from || OPENED[this.top]));
    history.replaceState({ ...(history.state as object | null), from: this.from }, "");
    this.arg = decoded(a);
    this.sub = decoded(b);
    this.page = decoded(c);
    this.visit++;
  }
}

export const route = new Route();
route.read();
addEventListener("hashchange", () => {
  route.read();
  scrollTo(0, 0);
  document.getElementById("main")?.scrollTo(0, 0);
});

/** The address of a page. */
export const href = (top: string, arg = "") => (arg ? `#/${top}/${encodeURIComponent(arg)}` : `#/${top}`);

export function go(top: string, arg = "") {
  location.hash = href(top, arg);
}

/** Searches for `q`: on the search page the address follows the field, without a step back for each key.
 *  A search's first letters are one step, so Back comes to the blank search page rather than before it. */
export function searchFor(q: string) {
  if (route.top !== "search") return go("search", q);
  if (!route.arg.trim() && q.trim()) history.pushState(null, "", href("search", q));
  else history.replaceState(null, "", href("search", q));
  route.arg = q;
}
