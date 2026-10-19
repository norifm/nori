// The page's colors: the core's season palette (the `season` and appearance settings, the date and the
// browser's zone) written into CSS custom properties on :root. Worked out again on load, when the page
// becomes visible again, when the system's color scheme changes and when a setting changes; never on a timer.
import { ask } from "../rpc";

/** What the core hands over (`Nori::theme`): `#rrggbb` colors. */
type Colors = { season: string; dark: boolean; page: string; surface: string; ink: string; ink2: string; hair: string; outline: string; accent: string; on: string; onAccent: string };

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

/** The calendar's season now (SPRING ... WINTER), as the core last said. */
export const worn = { season: "AUTUMN" };

function apply(c: Colors) {
  worn.season = c.season;
  const root = document.documentElement;
  for (const [k, v] of VARS) root.style.setProperty(v, c[k] as string);
  root.style.colorScheme = c.dark ? "dark" : "light";
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
  const now = new Date();
  const dark = matchMedia("(prefers-color-scheme: dark)").matches;
  const c = JSON.parse(await ask<string>("theme", now.getMonth() + 1, now.getDate(), Intl.DateTimeFormat().resolvedOptions().timeZone, dark)) as Colors;
  apply(c);
  // After the first colors are in, so they do not fade in from the system's.
  requestAnimationFrame(() => document.documentElement.classList.add("themed"));
  try {
    localStorage.setItem(KEY, JSON.stringify(c));
  } catch {
    /* private window */
  }
}

/** Wears the colors from last time and keeps them current once the core is up (`refreshTheme`). */
export function startTheme() {
  remembered();
  document.addEventListener("visibilitychange", () => !document.hidden && void refreshTheme().catch(() => {}));
  matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => void refreshTheme().catch(() => {}));
}
