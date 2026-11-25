<script lang="ts">
  // The first run, while the install has no profile: where the music is, any number of sources picked, then
  // Continue (the core's `WelcomeFlow`). A browser plays from its own kind only: a provider plugin can't run
  // here. Pair a device brings another device's profiles instead.
  import { W } from "../words";
  import Icon from "./Icon.svelte";

  let { picked, toggle, next, pair }: { picked: string[]; toggle: (source: string) => void; next: () => void; pair: () => void } = $props();

  /** What a browser plays from, by its kind's id, with its glyph and words. */
  const sources = [{ id: "subsonic", icon: "server", name: W.subsonic, about: W.subsonicAbout }];
</script>

<div id="welcome" class="welcome">
  <div class="body">
    <h1>{W.welcomeTitle}</h1>
    <div class="group-label">{W.welcomeWhere}</div>
    <div class="group">
      {#each sources as s (s.id)}
        {@const on = picked.includes(s.id)}
        <button class="group-row" data-kind={s.id} role="checkbox" aria-checked={on} onclick={() => toggle(s.id)}>
          <Icon name={s.icon} />
          <span class="words"><span>{s.name}</span><span class="sub">{s.about}</span></span>
          <span class="mark" class:on>{#if on}<Icon name="check" />{/if}</span>
        </button>
      {/each}
    </div>
    <button class="text-action" id="welcome-pair" onclick={pair}>{W.syncPair}</button>
  </div>
  <div class="end"><button class="pill" id="welcome-continue" disabled={!picked.length} onclick={next}>{W.continueOn}</button></div>
</div>

<style>
  .welcome {
    height: 100dvh;
    display: flex;
    flex-direction: column;
  }
  .body,
  .end {
    width: 100%;
    max-width: 480px;
    margin: 0 auto;
    padding: 0 var(--gutter);
  }
  .body {
    flex: 1;
    overflow-y: auto;
    padding-top: 56px;
  }
  h1 {
    font-size: 38px;
    line-height: 44px;
  }
  .group-label {
    margin-top: 32px;
  }
  /* A source's pick: an empty ring, or picked, a disc in the accent with a check. */
  .mark {
    flex: none;
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    border-radius: 50%;
    box-shadow: inset 0 0 0 1.5px color-mix(in srgb, var(--ink) 30%, transparent);
  }
  .mark.on {
    background: var(--accent);
    box-shadow: none;
    color: var(--on-accent);
  }
  .mark :global(.icon) {
    width: 14px;
    height: 14px;
    margin: 0;
  }
  .end {
    flex: none;
    padding-top: 12px;
    padding-bottom: calc(16px + env(safe-area-inset-bottom));
  }
  .end .pill {
    width: 100%;
  }
</style>
