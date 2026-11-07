// A page's newer answer replacing what it showed (Android's `itemMotion`): kept items move to their new
// places, new ones fade in, gone ones fade out. nori's Motion setting (the `still` class) makes each direct.
import { flip } from "svelte/animate";
import { cubicOut } from "svelte/easing";
import { fade } from "svelte/transition";

const still = () => document.documentElement.classList.contains("still");

/** `animate:settle` on a keyed each block's element. */
export const settle = (node: Element, places: { from: DOMRect; to: DOMRect }) => flip(node, places, { duration: still() ? 0 : 320, easing: cubicOut });

/** `transition:come` on the same element. */
export const come = (node: Element) => fade(node, { duration: still() ? 0 : 180 });
