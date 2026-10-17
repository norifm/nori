<script lang="ts">
  // AutoMix, speed and pitch, set where the session keeps them.
  import { player } from "../state/player.svelte";
  import { W } from "../words";

  const automix = $derived(player.sound.autoMix === "true");
</script>

{#if player.sound.speed !== undefined}
  <div class="sound">
    <button class="chip" class:on={automix} aria-pressed={automix} onclick={() => void player.setSound("autoMix", automix ? "false" : "true")}>{W.autoMix}</button>
    {#each [["speed", W.speed], ["pitch", W.pitch]] as [name, label] (name)}
      <select class="chip" aria-label={label} onchange={(e) => void player.setSound(name, e.currentTarget.value)}>
        {#each player.rates[name] ?? [] as v (v)}
          <option value={v} selected={Number(v) === Number(player.sound[name])}>{W.rate(label, v)}</option>
        {/each}
      </select>
    {/each}
  </div>
{/if}

<style>
  .sound {
    display: flex;
    justify-content: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .chip {
    appearance: none;
    border: 0;
    height: 28px;
    padding: 0 12px;
    border-radius: 14px;
    background: var(--fill);
    color: var(--dim);
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
    text-align: center;
  }
  .chip.on {
    background: var(--accent);
    color: var(--accent-text);
  }
</style>
