// A page's newer answer replacing what it showed (Android's `itemMotion`): kept items move to their new
// places, new ones fade in, gone ones fade out. nori's Motion setting (the `still` class) makes each direct.
import { flip } from "svelte/animate";
import { cubicOut } from "svelte/easing";
import { fade, fly } from "svelte/transition";

/** nori's Motion setting asks for less. */
export const still = () => document.documentElement.classList.contains("still");

/** A read answering this soon is shown without placeholders (the core's `stage().quick_load_ms`); what a
 * page read fades in over its placeholders in `EASE_IN_MS`. */
export const QUICK_LOAD_MS = 250;
export const EASE_IN_MS = 180;

/** `animate:settle` on a keyed each block's element. */
export const settle = (node: Element, places: { from: DOMRect; to: DOMRect }) => flip(node, places, { duration: still() ? 0 : 320, easing: cubicOut });

/** `transition:come` on the same element. */
export const come = (node: Element) => fade(node, { duration: still() ? 0 : 180 });

/** `in:arrive={{ way }}` on a first-run page: it comes a little from the side moved toward (1 forward, -1 back). */
export const arrive = (node: Element, { way }: { way: number }) => fly(node, { x: 40 * way, duration: still() ? 0 : 260, easing: cubicOut });
