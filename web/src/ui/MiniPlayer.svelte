<script lang="ts">
  // A phone's mini player: the top row of the bottom bar, over the tabs; a tap opens the full player.
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
    }}><Icon name={player.playing ? "pause-solid" : "play-solid"} /></button
  >
  <button
    class="round"
    aria-label={W.next}
    onclick={(e) => {
      e.stopPropagation();
      void player.control("next");
    }}><Icon name="forward-solid" /></button
  >
  <div class="line"><Seek thin times={false} /></div>
</div>

<style>
  .mini {
    position: relative;
    height: 56px;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 8px 0 10px;
    cursor: pointer;
    overflow: hidden;
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
  }
  .title {
    font-size: 14px;
    font-weight: 600;
  }
  .sub {
    color: var(--ink2);
    font-size: 12px;
  }
  .round :global(.icon) {
    width: 26px;
    height: 26px;
  }
  .line {
    position: absolute;
    left: 14px;
    right: 14px;
    bottom: 0;
  }
</style>
