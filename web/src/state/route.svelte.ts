// The page shown, by its address (`#/album/<id>`, `#/artist/<id>/<shelf>`): read from the hash whenever it
// changes.

/** The pages under the Library tab. */
const LIBRARY = ["library", "album", "artist", "playlist", "genre", "favorites"];

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
  /** A third part of the address: an artist's shelf. */
  more = $state("");
  /** Bumped by every visit, the same address again too: a page reads itself again. */
  visit = $state(0);

  /** The tab the page sits under. */
  tab = $derived(LIBRARY.includes(this.top) ? "library" : this.top === "row" ? "home" : this.top);

  read() {
    const [, top = "", a = "", b = ""] = location.hash.replace(/^#\/?/, "/").split("/");
    this.top = top || "home";
    this.arg = decoded(a);
    this.more = decoded(b);
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
export const href = (top: string, arg = "", more = "") =>
  `#/${top}${arg ? `/${encodeURIComponent(arg)}` : ""}${more ? `/${encodeURIComponent(more)}` : ""}`;

export function go(top: string, arg = "", more = "") {
  location.hash = href(top, arg, more);
}

/** Searches for `q`: on the search page the address follows the field, without a step back for each key. */
export function searchFor(q: string) {
  if (route.top !== "search") return go("search", q);
  history.replaceState(null, "", href("search", q));
  route.arg = q;
}
