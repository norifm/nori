<script lang="ts">
  // A phone's mini player, over the tabs in the one outline: the cover, what plays, where it plays, the
  // seal and play or pause. A click opens the full player. No progress bar: it would tick for as long as
  // the page is open.
  import { player } from "../state/player.svelte";
  import { ui } from "../state/ui.svelte";
  import { W } from "../words";
  import Cover from "./Cover.svelte";
  import Icon from "./Icon.svelte";
  import OutputButton from "./OutputButton.svelte";
  import Seal from "./Seal.svelte";

  const s = $derived(player.song!);
  const n = $derived(player.now);
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<div class="mini" id="mini" role="button" tabindex="0" aria-label={W.nowPlayingBar} onclick={() => (ui.full = true)}>
  <div class="art"><Cover src={s.cover} radius="10px" /></div>
  <div class="text">
    <span class="title clip">{s.title}</span>
    <span class="line clip">{s.artist}</span>
  </div>
  {#if !n?.asks}
    <OutputButton />
    <Seal what="song" id={s.id} on={!!s.starred} />
  {/if}
  <button
    class="glyph"
    aria-label={player.playing ? W.pause : W.play}
    onclick={(e) => {
      e.stopPropagation();
      void player.control("toggle");
    }}><Icon name={player.playing ? "pause-solid" : "play-solid"} /></button
  >
</div>

<style>
  .mini {
    display: flex;
    align-items: center;
    padding: 8px 4px 8px 8px;
    cursor: pointer;
  }
  .art {
    width: 40px;
    flex: none;
  }
  .text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    padding: 0 11px;
  }
  .title {
    font-size: 16px;
    line-height: 21px;
  }
  .line {
    font-size: 13px;
    line-height: 17px;
    color: color-mix(in srgb, var(--ink) 65%, transparent);
  }
  .mini :global(.output),
  .mini :global(.seal) {
    color: color-mix(in srgb, var(--ink) 75%, transparent);
  }
  .mini :global(.output.lit) {
    color: var(--ink);
  }
  .mini :global(.seal.on) {
    color: var(--accent);
  }
  /* The line glyphs (where it plays, the seal) one size, so their lines match; play and pause, solid, larger. */
  .mini :global(.output .icon),
  .mini :global(.seal .icon) {
    width: 22px;
    height: 22px;
  }
  .glyph :global(.icon) {
    width: 26px;
    height: 26px;
  }
</style>
