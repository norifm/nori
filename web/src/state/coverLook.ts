// A cover's page, as Android's album, artist and playlist pages and its player wear one: the page decodes a
// small copy of the cover, the core works out its colors on the season's paper and the wash (`cover_look`).
// Midnight wears no cover colors: then there is none.
import { ask } from "../rpc";
import { today } from "./theme.svelte";

const SIDE = 64;

/** The CSS custom properties to wear (the page's roles, the artwork's dissolve), the cover blurred and dyed to
 * lie behind the player, the same undyed for a page's head, and where the player's sleeve starts to fade. */
export type CoverLook = { vars: string; wash: ImageData | null; own: ImageData | null; melt: number };

type Colors = {
  page: string;
  surface: string;
  ink: string;
  ink2: string;
  outline: string;
  hair: string;
  accent: string;
  onAccent: string;
  heroEdge: string;
  heroMid: string;
  heroStops: number[];
  melt: number;
  washSide: number;
};

/** The look for the cover at `src`, or null when it cannot be read or the appearance wears none. */
export async function coverLook(src: string): Promise<CoverLook | null> {
  try {
    const answer = await fetch(src);
    if (!answer.ok) return null;
    const bitmap = await createImageBitmap(await answer.blob(), { resizeWidth: SIDE, resizeHeight: SIDE, resizeQuality: "medium" });
    const canvas = new OffscreenCanvas(SIDE, SIDE);
    const g = canvas.getContext("2d")!;
    g.drawImage(bitmap, 0, 0);
    bitmap.close();
    const rgba = new Uint8Array(g.getImageData(0, 0, SIDE, SIDE).data.buffer);
    const [json, wash, own] = await ask<[string | null, Uint8ClampedArray | null, Uint8ClampedArray | null]>("cover_look", rgba, SIDE, SIDE, ...today());
    if (!json) return null;
    const c = JSON.parse(json) as Colors;
    const [a, b, m] = c.heroStops.map((s) => `${(s * 100).toFixed(1)}%`);
    const dissolve = `linear-gradient(to bottom, transparent ${a}, ${c.heroEdge} ${b}, ${c.heroMid} ${m}, ${c.page})`;
    const vars = `--page:${c.page};--surface:${c.surface};--ink:${c.ink};--ink2:${c.ink2};--outline:${c.outline};--hair:${c.hair};--accent:${c.accent};--on-accent:${c.onAccent};--dissolve:${dissolve}`;
    const image = (px: Uint8ClampedArray | null) => (px && c.washSide ? new ImageData(new Uint8ClampedArray(px), c.washSide, c.washSide) : null);
    return { vars, wash: image(wash), own: image(own), melt: c.melt };
  } catch {
    return null;
  }
}
