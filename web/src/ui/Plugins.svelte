<script lang="ts">
  // Settings › Plugins: what's installed, each with its page of rows, and nori's repo. What can run here
  // is the core's to say (`web_fit`); a plugin that can't is greyed with its reasons.
  import { ask } from "../rpc";
  import { ui } from "../state/ui.svelte";
  import { W } from "../words";
  import type { Installed, Row, Value } from "../plugins/types";
  import { valueOf } from "../plugins/types";

  type Fit = { state: "Supported" | "Unknown" } | { state: "Blocked"; blocks: string[] };
  type Offer = { id: string; name: string; about: string | null; icon: string | null; version: string | null; url: string | null; sha256: string | null; fit: Fit };
  let offers = $state.raw<Offer[] | null>(null);
  let failed = $state(false);
  let busy = $state("");
  let open = $state("");
  let host = $state.raw<typeof import("../plugins/host.svelte").plugins | null>(null);

  void (async () => {
    const { plugins } = await import("../plugins/host.svelte");
    await plugins.load();
    host = plugins;
    await ask<string>("plugin_offers", plugins.proxied).then(
      (j) => (offers = JSON.parse(j)),
      () => (failed = true),
    );
  })();

  const lines = (f: Fit) => (f.state === "Blocked" ? f.blocks.map((b) => W.webBlocks[b] ?? b) : f.state === "Unknown" ? [W.pluginFit.Unknown] : []);
  const page = $derived(host?.installed.find((i) => i.plugin.id === open));
  const running = $derived(page && host?.runningOf(page.plugin.id));
  const number = (i: Installed, r: Row) => Number(valueOf(r, i.values) ?? 0);

  async function install(key: string, get: () => Promise<Uint8Array>, sha256?: string) {
    busy = key;
    try {
      const blocks = await host!.install(await get(), sha256);
      if (blocks.length > 0) ui.say(blocks.map((b) => W.webBlocks[b] ?? b).join(", "));
    } catch (e) {
      console.warn(`plugin install: ${e}`);
      ui.say(W.pluginInstallFailed);
    }
    busy = "";
  }

  function fromFile(e: Event) {
    const file = (e.target as HTMLInputElement).files?.[0];
    if (file) void install("file", async () => new Uint8Array(await file.arrayBuffer()));
  }

  const set = (i: Installed, r: Row, v: Value) => void host!.setRow(i.plugin.id, r, v);
</script>

{#if page}
  {@const i = page}
  <div class="group-label">{i.plugin.name}</div>
  {#if i.plugin.about}<p class="quiet">{i.plugin.about}</p>{/if}
  <div class="group" data-plugin-page={i.plugin.id}>
    <label class="group-row">
      <span class="words">{W.pluginOn}</span>
      <input type="checkbox" class="switch" aria-label={W.pluginOn} data-plugin-on checked={i.on} onchange={(e) => void host!.setOn(i.plugin.id, e.currentTarget.checked)} />
    </label>
    {#each i.plugin.rows as r (r.id)}
      {@const live = running?.shown[r.id]}
      {#if live?.shown !== false}
        {#if r.kind === "Switch"}
          <label class="group-row" class:off={live?.enabled === false} data-row={r.id}>
            <span class="words"><span>{r.label}</span>{#if r.hint}<span class="sub">{r.hint}</span>{/if}</span>
            <input type="checkbox" class="switch" aria-label={r.label} disabled={live?.enabled === false} checked={number(i, r) !== 0} onchange={(e) => set(i, r, e.currentTarget.checked ? 1 : 0)} />
          </label>
        {:else if r.kind === "Choice"}
          <div class="group-row" data-row={r.id}>
            <span class="words"><span>{r.label}</span>{#if r.hint}<span class="sub">{r.hint}</span>{/if}</span>
            <select aria-label={r.label} value={number(i, r)} onchange={(e) => set(i, r, Number(e.currentTarget.value))}>
              {#each r.options as o, n}<option value={n}>{o}</option>{/each}
            </select>
          </div>
        {:else if r.kind === "Level"}
          <div class="group-row level" data-row={r.id}>
            <span class="words"><span>{r.label}</span>{#if r.hint}<span class="sub">{r.hint}</span>{/if}</span>
            <input class="range" type="range" aria-label={r.label} min={r.min} max={r.max} step={r.step ?? "any"} value={number(i, r)} oninput={(e) => set(i, r, Number(e.currentTarget.value))} />
            <span class="sub">{number(i, r).toFixed(r.decimals)}{r.unit ? ` ${r.unit}` : ""}</span>
          </div>
        {:else if r.kind === "Text" || r.kind === "Secret"}
          <label class="group-row" data-row={r.id}>
            <span class="words">{r.label}</span>
            <input class="field" type={r.kind === "Secret" ? "password" : "text"} aria-label={r.label} value={valueOf(r, i.values) ?? ""} onchange={(e) => set(i, r, e.currentTarget.value)} />
          </label>
        {:else if r.kind === "Action"}
          <button class="group-row" data-row={r.id} onclick={() => void host!.act(i.plugin.id, r.id)}><span class="words">{r.label}</span></button>
        {:else if r.kind === "Link" && r.url}
          <a class="group-row" data-row={r.id} href={r.url} target="_blank" rel="noreferrer"><span class="words">{r.label}</span></a>
        {:else if r.kind === "Section"}
          <div class="group-row" data-row={r.id}><span class="sub">{r.label}</span></div>
        {:else}
          <div class="group-row" data-row={r.id}><span class="words"><span class="sub">{running?.status[r.id] ?? r.label}</span></span></div>
        {/if}
      {/if}
    {/each}
    <button class="group-row" data-plugin-remove onclick={() => { const id = i.plugin.id; open = ""; void host!.remove(id); }}><span class="words">{W.pluginRemove}</span></button>
  </div>
  <button class="link" onclick={() => (open = "")}>{W.plugins}</button>
{:else}
  {#if host && host.installed.length > 0}
    <div class="group-label">{W.pluginsInstalled}</div>
    <div class="group">
      {#each host.installed as i (i.plugin.id)}
        <button class="group-row plugin" data-installed={i.plugin.id} onclick={() => (open = i.plugin.id)}>
          <span class="tile">{#if i.icon}<img src={i.icon} alt="" />{:else}{i.plugin.name.slice(0, 1).toUpperCase()}{/if}</span>
          <span class="words">
            <span class="clip">{i.plugin.name}</span>
            <span class="sub">{i.on ? W.pluginOn : W.pluginIsOff} · {i.plugin.version}</span>
          </span>
        </button>
      {/each}
    </div>
  {/if}

  <div class="group-label">{W.pluginsGet}</div>
  {#if failed}
    <p class="quiet">{W.pluginRepoUnreachable}</p>
  {:else if !offers}
    <p class="quiet">{W.pluginsAsking}</p>
  {:else}
    <div class="group">
      {#each offers as o (o.id)}
        {@const has = host?.find(o.id)}
        <div class="group-row plugin" class:off={o.fit.state === "Blocked"} data-plugin={o.id} data-fit={o.fit.state}>
          <span class="tile">{#if o.icon}<img src={o.icon} alt="" />{:else}{o.name.slice(0, 1).toUpperCase()}{/if}</span>
          <span class="words">
            <span class="clip">{o.name}</span>
            {#if o.about}<span class="sub">{o.about}</span>{/if}
            {#if o.version}<span class="sub">{o.version}</span>{/if}
            {#each lines(o.fit) as line}<span class="sub why">{line}</span>{/each}
          </span>
          {#if o.fit.state !== "Blocked" && o.url && host?.proxied && has?.plugin.version !== o.version}
            <button class="pill" data-install disabled={busy !== ""} onclick={() => void install(o.id, () => host!.download(o.url!), o.sha256!)}>{busy === o.id ? W.pluginInstalling : W.pluginInstall}</button>
          {/if}
        </div>
      {/each}
    </div>
  {/if}
  <label class="link">
    {W.pluginFromFile}
    <input type="file" accept=".noriplugin" hidden disabled={!host || busy !== ""} onchange={fromFile} />
  </label>
{/if}

<style>
  .plugin {
    align-items: flex-start;
    gap: 12px;
    padding-top: 10px;
    padding-bottom: 10px;
    text-align: left;
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
  .pill {
    flex: none;
    align-self: center;
    padding: 6px 14px;
    border-radius: 999px;
    border: 1px solid var(--line, var(--hover));
    background: none;
    color: var(--ink);
    font: inherit;
    font-size: 13px;
    cursor: pointer;
  }
  .pill:disabled {
    opacity: 0.55;
  }
  .link {
    display: block;
    margin: 12px 4px;
    padding: 0;
    border: 0;
    background: none;
    color: var(--ink2);
    font: inherit;
    font-size: 13px;
    cursor: pointer;
  }
  .field,
  select {
    max-width: 50%;
    padding: 4px 8px;
    border-radius: 8px;
    border: 1px solid var(--hover);
    background: none;
    color: var(--ink);
    font: inherit;
  }
  .level .range {
    flex: 1;
    min-width: 0;
  }
</style>
