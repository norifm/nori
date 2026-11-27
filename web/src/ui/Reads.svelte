<script lang="ts">
  // Where a page shows what it is still reading (`reading`, a `sketch::Reading` by name; `known`: its head
  // shows already): past the core's quick-load time, the sketch of what will stand there (Sketch), fading
  // in; once `ready`, what follows this place fades in over the sketch as it fades out, and nothing moves.
  // An answer inside the quick-load time is simply there. nori's Motion setting asking for less makes each
  // change direct. Its place is a box of no height, so the page's margins and its own rules run as before.
  import { tick } from "svelte";
  import { fade } from "svelte/transition";
  import Sketch from "./Sketch.svelte";
  import { EASE_IN_MS, QUICK_LOAD_MS, still } from "./motion";

  let { ready, reading, known = false }: { ready: boolean; reading: string; known?: boolean } = $props();

  let at = $state<HTMLElement>();
  /** The read outlasted the quick-load time: the sketch shows, and the answer will fade in over it. */
  let late = $state(false);
  $effect(() => {
    if (ready) return;
    late = false;
    const wait = setTimeout(() => (late = true), QUICK_LOAD_MS);
    return () => clearTimeout(wait);
  });
  $effect(() => {
    if (!ready || !late || still()) return;
    void tick().then(() => {
      for (let n = at?.nextElementSibling; n; n = n.nextElementSibling) n.animate({ opacity: [0, 1] }, { duration: EASE_IN_MS, easing: "ease-out" });
    });
  });
  const shown = () => ({ duration: still() ? 0 : EASE_IN_MS });
</script>

<div class="reads" bind:this={at}>
  {#if late && !ready}<div class="sketch" transition:fade={shown()}><Sketch {reading} {known} /></div>{/if}
</div>

<style>
  .reads {
    position: relative;
  }
  /* Over the page where what it reads will stand, cut at the window's foot. */
  .sketch {
    position: absolute;
    inset: 0 0 auto;
    max-height: 100vh;
    overflow: hidden;
    pointer-events: none;
  }
</style>
