// Now Playing in its cover's colors, as Android's player: the page decodes a small copy of the cover and
// the core picks the colors and the wash (`cover_look`). Worked out once per cover.
import { ask } from "../rpc";

const SIDE = 64;

/** The CSS custom properties to wear, and the cover blurred to lie behind them (null on none). */
export type CoverLook = { vars: string; wash: ImageData | null };

type Colors = { page: string; surface: string; ink: string; ink2: string; outline: string; hair: string; accent: string; onAccent: string; washSide: number };

/** The look for the cover at `src`, or null when it cannot be read (the page's own colors stay). */
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
    const [json, wash] = await ask<[string, Uint8ClampedArray | null]>("cover_look", rgba, SIDE, SIDE);
    const c = JSON.parse(json) as Colors;
    const vars = `--page:${c.page};--surface:${c.surface};--ink:${c.ink};--ink2:${c.ink2};--outline:${c.outline};--hair:${c.hair};--accent:${c.accent};--on-accent:${c.onAccent}`;
    return { vars, wash: wash && c.washSide ? new ImageData(new Uint8ClampedArray(wash), c.washSide, c.washSide) : null };
  } catch {
    return null;
  }
}
