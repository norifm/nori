<script lang="ts">
  // A phone's mini player over the tab bar; a tap opens the full player.
  import { player } from "../state/player.svelte";
  import { ui } from "../state/ui.svelte";
  import { W } from "../words";
  import Cover from "./Cover.svelte";
  import Icon from "./Icon.svelte";
  import Seek from "./Seek.svelte";

  const s = $derived(player.song!);
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<div class="mini" id="mini" role="button" tabindex="0" aria-label={s.title} onclick={() => (ui.full = true)}>
  <div class="art"><Cover src={s.cover} /></div>
  <div class="text">
    <span class="title clip">{s.title}</span>
    <span class="sub clip">{player.now?.device ? W.playingOn(player.now.device) : s.artist}</span>
  </div>
  <button
    class="round"
    aria-label={player.playing ? W.pause : W.play}
    onclick={(e) => {
      e.stopPropagation();
      void player.control("toggle");
    }}><Icon name={player.playing ? "pause" : "play"} /></button
  >
  <button
    class="round"
    aria-label={W.next}
    onclick={(e) => {
      e.stopPropagation();
      void player.control("next");
    }}><Icon name="next" /></button
  >
  <div class="line"><Seek thin times={false} /></div>
</div>

<style>
  .mini {
    position: fixed;
    left: 8px;
    right: 8px;
    bottom: calc(var(--tabs-h) + env(safe-area-inset-bottom) + 6px);
    height: var(--mini-h);
    z-index: 31;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 10px 0 8px;
    border-radius: 14px;
    background: var(--raised);
    box-shadow: var(--shadow);
    cursor: pointer;
    overflow: hidden;
  }
  .art {
    width: 44px;
    flex: none;
  }
  .text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .title {
    font-size: 14px;
    font-weight: 500;
  }
  .round :global(.icon) {
    width: 26px;
    height: 26px;
  }
  .line {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
  }
</style>
