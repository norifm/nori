<script lang="ts">
  // Settings › Plugins, as the desktop's browser of nori's repo. What can run here is the core's to say
  // (`web_fit`); a plugin that can't is greyed with its reasons. Nothing installs until the web has a host.
  import { ask } from "../rpc";
  import { W } from "../words";

  type Fit = { state: "Supported" | "Unknown" } | { state: "Blocked"; blocks: string[] };
  type Offer = { id: string; name: string; about: string | null; icon: string | null; version: string | null; fit: Fit };
  let offers = $state.raw<Offer[] | null>(null);
  let failed = $state(false);
  void ask<string>("plugin_offers").then(
    (j) => (offers = JSON.parse(j)),
    () => (failed = true),
  );
  const lines = (f: Fit) => (f.state === "Blocked" ? f.blocks.map((b) => W.webBlocks[b] ?? b) : [W.pluginFit[f.state]]);
</script>

<div class="group-label">{W.pluginsGet}</div>
{#if failed}
  <p class="quiet">{W.pluginRepoUnreachable}</p>
{:else if !offers}
  <p class="quiet">{W.pluginsAsking}</p>
{:else}
  <div class="group">
    {#each offers as o (o.id)}
      <div class="group-row plugin" class:off={o.fit.state === "Blocked"} data-plugin={o.id} data-fit={o.fit.state}>
        <span class="tile">{#if o.icon}<img src={o.icon} alt="" />{:else}{o.name.slice(0, 1).toUpperCase()}{/if}</span>
        <span class="words">
          <span class="clip">{o.name}</span>
          {#if o.about}<span class="sub">{o.about}</span>{/if}
          {#if o.version}<span class="sub">{o.version}</span>{/if}
          {#each lines(o.fit) as line}<span class="sub why">{line}</span>{/each}
        </span>
      </div>
    {/each}
  </div>
{/if}

<style>
  .plugin {
    align-items: flex-start;
    gap: 12px;
    padding-top: 10px;
    padding-bottom: 10px;
  }
  .plugin.off .tile,
  .plugin.off .words > :not(.why) {
    opacity: 0.55;
  }
  .tile {
    width: 44px;
    height: 44px;
    flex: none;
    display: grid;
    place-items: center;
    overflow: hidden;
    border-radius: 12px;
    background: var(--hover);
    color: var(--ink2);
    font-weight: 600;
  }
  .tile img {
    width: 100%;
    height: 100%;
  }
  .words {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .sub {
    font-size: 12.5px;
    color: var(--ink2);
  }
  .why {
    color: var(--ink);
  }
  .quiet {
    margin: 0 4px;
    font-size: 13px;
    color: var(--ink2);
  }
</style>
