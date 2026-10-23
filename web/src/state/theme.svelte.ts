// The page's colors: the core's season palette (the `season` and appearance settings, the date and the
// browser's zone) written into CSS custom properties on :root. Worked out again on load, when the page
// becomes visible again, when the system's color scheme changes and when a setting changes; never on a timer.
import { ask } from "../rpc";

/** What the core hands over (`Nori::theme`): `#rrggbb` colors. */
type Colors = { season: string; next: string; nextMonth: number; nextDay: number; dark: boolean; page: string; surface: string; ink: string; ink2: string; hair: string; outline: string; accent: string; on: string; onAccent: string; still?: boolean; tiles?: Tile[] };
/** A season's tile: its paper, accent and "on" color in the appearance worn. */
export type Tile = { season: string; page: string; accent: string; on: string };

const KEY = "nori-theme";
const VARS: [keyof Colors, string][] = [
  ["page", "--page"],
  ["surface", "--surface"],
  ["ink", "--ink"],
  ["ink2", "--ink2"],
  ["hair", "--hair"],
  ["outline", "--outline"],
  ["accent", "--accent"],
  ["on", "--on"],
  ["onAccent", "--on-accent"],
];

/** The calendar's season now (SPRING ... WINTER) and the next one with its first day, and the seasons'
 * tiles, as the core last said. */
export const worn = $state({ season: "AUTUMN", next: "WINTER", nextMonth: 11, nextDay: 7, tiles: [] as Tile[] });

/** What the core works a theme out from: the browser's date and zone, and its color scheme. */
export function today(): [number, number, string, boolean] {
  const now = new Date();
  return [now.getMonth() + 1, now.getDate(), Intl.DateTimeFormat().resolvedOptions().timeZone, matchMedia("(prefers-color-scheme: dark)").matches];
}

function apply(c: Colors) {
  // Colors remembered by an older page have no next season.
  Object.assign(worn, c.next ? { season: c.season, next: c.next, nextMonth: c.nextMonth, nextDay: c.nextDay } : { season: c.season }, c.tiles ? { tiles: c.tiles } : {});
  const root = document.documentElement;
  for (const [k, v] of VARS) root.style.setProperty(v, c[k] as string);
  root.style.colorScheme = c.dark ? "dark" : "light";
  // nori's Motion setting, as on every client: animations run unless it asks for less.
  root.classList.toggle("still", !!c.still);
  let meta = document.querySelector<HTMLMetaElement>('meta[name="theme-color"]');
  if (!meta) {
    meta = document.createElement("meta");
    meta.name = "theme-color";
    document.head.append(meta);
  }
  meta.content = c.page;
}

/** The colors worn last, so the first paint is not a flash of the system's. */
function remembered() {
  try {
    const c = JSON.parse(localStorage.getItem(KEY) ?? "null") as Colors | null;
    if (c) apply(c);
  } catch {
    /* no storage: the core's answer comes in a moment */
  }
}

/** Asks the core for the colors and wears them; later changes fade in (the `themed` class turns the transition on). */
export async function refreshTheme() {
  const c = JSON.parse(await ask<string>("theme", ...today())) as Colors;
  apply(c);
  looks.rev++;
  // After the first colors are in, so they do not fade in from the system's.
  requestAnimationFrame(() => document.documentElement.classList.add("themed"));
  try {
    localStorage.setItem(KEY, JSON.stringify(c));
  } catch {
    /* private window */
  }
}

/** Bumped when the theme changes: a cover's look is worked out again on the new paper. */
export const looks = $state({ rev: 0 });

/** Wears the colors from last time and keeps them current once the core is up (`refreshTheme`). */
export function startTheme() {
  remembered();
  document.addEventListener("visibilitychange", () => !document.hidden && void refreshTheme().catch(() => {}));
  matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => void refreshTheme().catch(() => {}));
}
