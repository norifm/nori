// The page shown, by its address (`#/album/<id>`): read from the hash whenever it changes.

/** Pages under the Library tab on a phone. */
const LIBRARY = ["library", "albums", "album", "artists", "artist", "playlists", "playlist", "genres", "genre", "songs", "favorites"];

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
  /** Bumped by every visit, the same address again too: a page reads itself again. */
  visit = $state(0);

  /** The tab the page sits under on a phone. */
  tab = $derived(LIBRARY.includes(this.top) ? "library" : this.top === "row" ? "home" : this.top === "equalizer" ? "settings" : this.top);

  read() {
    const [, top = "", a = ""] = location.hash.replace(/^#\/?/, "/").split("/");
    this.top = top || "home";
    this.arg = decoded(a);
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

/** Searches for `q`: on the search page the address follows the field, without a step back for each key. */
export function searchFor(q: string) {
  if (route.top !== "search") return go("search", q);
  history.replaceState(null, "", href("search", q));
  route.arg = q;
}
