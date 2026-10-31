<script lang="ts">
  // Settings › Plugins and the pages it opens, as Android's: the list (updates, what's installed, the switch
  // over them all), the browser (`#/settings/Plugins/get`) and a plugin's page (`#/settings/Plugins/<id>`),
  // installed or only offered. What can run here is the core's to say (`web_fit`); a plugin that can't is
  // greyed with its reasons. `tabbed`: the list under Settings' tabs, which give it its title.
  import { ask } from "../rpc";
  import { href, route } from "../state/route.svelte";
  import { ui } from "../state/ui.svelte";
  import { W } from "../words";
  import type { Installed, Offer, Read, Readme, Row, Value, Version } from "../plugins/types";
  import { valueOf } from "../plugins/types";
  import Icon from "./Icon.svelte";
  import Page from "./Page.svelte";

  let { tabbed = false }: { tabbed?: boolean } = $props();

  let offers = $state.raw<Offer[] | null>(null);
  let failed = $state(false);
  let busy = $state("");
  let query = $state("");
  let found = $state.raw<string[] | null>(null);
  let readme = $state.raw<Readme[]>([]);
  let host = $state.raw<typeof import("../plugins/host.svelte").plugins | null>(null);

  async function offered() {
    const stands = host!.installed.map((i) => ({ id: i.plugin.id, version: i.plugin.version, held: !!i.held, fromFile: !!i.fromFile }));
    await ask<string>("plugin_offers", host!.proxied, JSON.stringify(stands)).then(
      (j) => (offers = JSON.parse(j)),
      () => (failed = true),
    );
  }

  void (async () => {
    const { plugins } = await import("../plugins/host.svelte");
    await plugins.load();
    host = plugins;
    await offered();
  })();

  const GET = "get";
  const sub = (page: string) => `${href("settings", "Plugins")}/${encodeURIComponent(page)}`;
  const shown = $derived(route.top === "settings" ? route.sub : "");
  const id = $derived(shown === GET ? "" : shown);
  const card = $derived(id ? host?.find(id) : undefined);
  /** What the repo holds of plugin `id`; one installed from a file comes from no repo. */
  const offerOf = (id: string, i?: Installed) => (i?.fromFile ? undefined : offers?.find((o) => o.id === id));
  const offer = $derived(id ? offerOf(id, card) : undefined);
  const updates = $derived((offers ?? []).filter((o) => o.waiting));
  const running = $derived(card && host?.runningOf(card.plugin.id));
  const listed = $derived((found ?? []).flatMap((id) => offers?.find((o) => o.id === id) ?? []));
  const newest = $derived(offer?.versions[0]);

  $effect(() => {
    const q = query;
    if (offers) void ask<string>("plugin_found", JSON.stringify(offers.map((o) => ({ id: o.id, name: o.name, about: o.about }))), q).then((j) => (found = JSON.parse(j)));
  });

  // The package's README, else the repo's.
  $effect(() => {
    readme = [];
    const [text, url] = [card?.readme, offer?.readme];
    void (async () => {
      const markdown = text ?? (url && host ? new TextDecoder().decode(await host.download(url).catch(() => new Uint8Array())) : "");
      if (markdown) readme = JSON.parse(await ask<string>("plugin_readme", markdown));
    })();
  });

  const lines = (o: Offer) => (o.fit.state === "Blocked" ? o.fit.blocks.map((b) => W.webBlocks[b] ?? b) : o.fit.state === "Unknown" ? [W.pluginFit.Unknown] : []);
  const blocked = (o?: Offer) => o?.fit.state === "Blocked";
  const dotted = (...parts: (string | false | undefined | null)[]) => parts.filter(Boolean).join(" · ");
  const number = (i: Installed, r: Row) => Number(valueOf(r, i.values) ?? 0);
  const letter = (name: string) => name.slice(0, 1).toUpperCase();
  const host_ = (url: string) => URL.parse(url)?.hostname ?? url;
  const levelShown = (i: Installed, r: Row) => {
    const v = number(i, r).toFixed(r.decimals);
    return r.unit ? `${v} ${r.unit}` : v;
  };

  /** Reads a package, shows what it is and may reach, and installs it when the user agrees. */
  async function get(key: string, pack: () => Promise<Uint8Array>, sha256?: string, fromFile = false) {
    busy = key;
    try {
      const read: Read = await host!.read(await pack(), sha256);
      const p = read.plugin;
      if (p.blocks.length > 0) ui.say(p.blocks.map((b) => W.webBlocks[b] ?? b).join(", "));
      else {
        const about = [p.about, dotted(W.pluginVersion(p.version), W.pluginKinds(p.points, p.overrides.length > 0), fromFile ? W.pluginFromFile : W.pluginRepoOfficial)];
        const may = W.pluginMay(p);
        const line = [...about, ...(may.length ? [W.pluginMayTitle, ...may.map((l) => `· ${l}`)] : [])].filter(Boolean).join("\n");
        if (await ui.confirm(p.name, line, W.pluginInstall)) {
          await host!.install(read, fromFile);
          await offered();
        }
      }
    } catch (e) {
      console.warn(`plugin install: ${e}`);
      ui.say(W.pluginInstallFailed);
    }
    busy = "";
  }

  const getVersion = (o: Offer, v: Version) => void get(o.id, () => host!.download(v.url), v.sha256);

  function fromFile(e: Event) {
    const file = (e.target as HTMLInputElement).files?.[0];
    if (file) void get("file", async () => new Uint8Array(await file.arrayBuffer()), undefined, true);
  }

  const set = (i: Installed, r: Row, v: Value) => void host!.setRow(i.plugin.id, r, v);

  async function remove(i: Installed) {
    if (!(await ui.confirm(W.pluginRemoveTitle(i.plugin.name), W.pluginRemoveText, W.pluginRemove))) return;
    const id = i.plugin.id;
    history.back();
    await host!.remove(id);
    await offered();
  }

  function menu(e: MouseEvent, i: Installed) {
    const at = host!.installed.findIndex((x) => x.plugin.id === i.plugin.id);
    ui.drop(e, [
      ...(at > 0 ? [{ label: W.pluginMoveUp, run: () => host!.move(i.plugin.id, -1) }] : []),
      ...(at < host!.installed.length - 1 ? [{ label: W.pluginMoveDown, run: () => host!.move(i.plugin.id, 1) }] : []),
      { label: W.pluginRemove, run: () => remove(i) },
    ]);
  }

  /** A card's rows as nori's own settings pages draw theirs: each section the label over the group after it. */
  function groups(i: Installed) {
    const out: { label: string | null; rows: Row[] }[] = [{ label: null, rows: [] }];
    for (const r of i.plugin.rows) {
      if (running?.shown[r.id]?.shown === false) continue;
      if (r.kind === "Section") out.push({ label: r.label, rows: [] });
      else out[out.length - 1].rows.push(r);
    }
    return out.filter((g) => g.rows.length > 0);
  }
</script>

{#snippet tile(name: string, icon: string | null | undefined, big = false)}
  <span class="tile" class:big>{#if icon}<span class="mark" style:mask-image="url({icon})"></span>{:else}{letter(name)}{/if}</span>
{/snippet}

{#snippet line(page: string, name: string, icon: string | null | undefined, about: string | null, quiet: string, attrs: Record<string, string>)}
  <a class="hit" href={sub(page)} {...attrs}>
    {@render tile(name, icon)}
    <span class="words">
      <span class="clip">{name}</span>
      {#if about}<span class="sub about">{about}</span>{/if}
      <span class="sub tiny clip">{quiet}</span>
    </span>
  </a>
{/snippet}

{#snippet tools()}
  {#if host && host.installed.length > 0}
    <input type="checkbox" class="switch" aria-label={W.pluginsEvery} title={W.pluginsEvery} data-plugins-every checked={!host.allOff} onchange={(e) => void host!.setAllOff(!e.currentTarget.checked)} />
  {/if}
  <a class="glyph" href={sub(GET)} aria-label={W.pluginsGet} title={W.pluginsGet} data-plugins-get><Icon name="plus" /></a>
{/snippet}

{#snippet list()}
  {#if host && host.installed.length === 0}
    <p class="quiet blurb">{W.pluginsEmpty}</p>
    <a class="pill wide" href={sub(GET)}>{W.pluginsGet}</a>
  {:else if host}
    {#if updates.length > 0}
      <div class="group-label">{W.pluginsUpdates}</div>
      <div class="group">
        {#each updates as o (o.id)}
          {@const i = host.find(o.id)}
          <div class="group-row plugin" data-update={o.id}>
            {@render line(o.id, o.name, i?.icon ?? o.icon, null, W.pluginUpdateLine(i?.plugin.version ?? "", o.versions[0].version), {})}
            <button class="text-action" disabled={busy !== ""} onclick={() => getVersion(o, o.versions[0])}>{W.pluginUpdate}</button>
          </div>
        {/each}
      </div>
    {/if}
    <div class="group-label">{W.pluginsInstalled}</div>
    <div class="group" class:off={host.allOff}>
      {#each host.installed as i (i.plugin.id)}
        <div class="group-row plugin">
          {@render line(i.plugin.id, i.plugin.name, i.icon, i.plugin.about, dotted(i.plugin.version, W.pluginKinds(i.plugin.points, i.plugin.overrides.length > 0)), { "data-installed": i.plugin.id })}
          <input type="checkbox" class="switch" aria-label={i.plugin.name} data-plugin-switch disabled={host.allOff} checked={i.on} onchange={(e) => void host!.setOn(i.plugin.id, e.currentTarget.checked)} />
        </div>
      {/each}
    </div>
  {/if}
{/snippet}

{#snippet browser()}
  <div class="search-field find">
    <Icon name="search" />
    <input type="search" bind:value={query} placeholder={W.pluginsSearch} aria-label={W.pluginsSearch} autocomplete="off" />
  </div>
  {#if failed}
    <p class="quiet">{W.pluginRepoUnreachable}</p>
  {:else if !offers || !found}
    <p class="quiet">{W.pluginsAsking}</p>
  {:else if listed.length === 0}
    <p class="quiet">{W.pluginsNoneFound}</p>
  {:else}
    <div class="group">
      {#each listed as o (o.id)}
        {@const v = o.versions[0]}
        <div class="group-row plugin" class:off={blocked(o)} data-plugin={o.id} data-fit={o.fit.state}>
          {@render line(o.id, o.name, o.icon, o.about, dotted(v?.version, W.pluginRepoOfficial), {})}
          {#each lines(o) as why}<span class="sub why">{why}</span>{/each}
          {#if o.state === "Installed"}
            <span class="value" data-up-to-date>{W.pluginUpToDate}</span>
          {:else if !blocked(o) && v && host?.proxied}
            <button class="text-action" data-install disabled={busy !== ""} onclick={() => getVersion(o, v)}>{busy === o.id ? W.pluginInstalling : o.state === "Update" ? W.pluginUpdate : W.pluginInstall}</button>
          {/if}
        </div>
      {/each}
    </div>
  {/if}
{/snippet}

{#snippet rowOf(i: Installed, r: Row)}
  {@const live = running?.shown[r.id]}
  {@const off = live?.enabled === false}
  {#if r.kind === "Switch"}
    <label class="group-row" class:off data-row={r.id}>
      <span class="words"><span>{r.label}</span>{#if r.hint}<span class="sub">{r.hint}</span>{/if}</span>
      <input type="checkbox" class="switch" aria-label={r.label} disabled={off} checked={number(i, r) !== 0} onchange={(e) => set(i, r, e.currentTarget.checked ? 1 : 0)} />
    </label>
  {:else if r.kind === "Choice"}
    <button
      class="group-row"
      class:off
      data-row={r.id}
      disabled={off}
      onclick={(e) => ui.drop(e, r.options.map((o, n) => ({ label: o, chosen: n === number(i, r), run: () => set(i, r, n) })))}
      ><span class="words"><span>{r.label}</span>{#if r.hint}<span class="sub">{r.hint}</span>{/if}</span><span class="value">{r.options[number(i, r)]}</span><Icon name="nav-arrow-right" /></button
    >
  {:else if r.kind === "Level"}
    {@const [min, max] = [r.min ?? 0, r.max ?? 1]}
    <div class="group-row level" class:off data-row={r.id}>
      <span class="words"><span class="sub level-line">{W.pluginLevel(r.label, levelShown(i, r))}</span>{#if r.hint}<span class="sub">{r.hint}</span>{/if}</span>
      <input class="range" type="range" aria-label={r.label} disabled={off} {min} {max} step={r.step ?? "any"} value={number(i, r)} style:--v="{((number(i, r) - min) / (max - min || 1)) * 100}%" oninput={(e) => set(i, r, Number(e.currentTarget.value))} />
    </div>
  {:else if r.kind === "Text" || r.kind === "Secret"}
    <label class="group-row text" class:off data-row={r.id}>
      <span class="sub">{r.label}</span>
      <input class="field" type={r.kind === "Secret" ? "password" : "text"} aria-label={r.label} disabled={off} value={valueOf(r, i.values) ?? ""} onchange={(e) => set(i, r, e.currentTarget.value)} />
      {#if r.hint}<span class="sub">{r.hint}</span>{/if}
    </label>
  {:else if r.kind === "Action"}
    <button class="group-row" class:off data-row={r.id} disabled={off} onclick={() => void host!.act(i.plugin.id, r.id)}><span class="words"><span>{r.label}</span>{#if r.hint}<span class="sub">{r.hint}</span>{/if}</span></button>
  {:else if r.kind === "Link" && r.url}
    <a class="group-row" class:off data-row={r.id} href={r.url} target="_blank" rel="noreferrer"><span class="words"><span>{r.label}</span>{#if r.hint}<span class="sub">{r.hint}</span>{/if}</span><span class="value">{host_(r.url)}</span><Icon name="nav-arrow-right" /></a>
  {:else if r.kind === "Status"}
    <div class="group-row" class:off data-row={r.id}><span class="words"><span>{r.label}</span>{#if r.hint}<span class="sub">{r.hint}</span>{/if}</span>{#if running?.status[r.id]}<span class="value">{running.status[r.id]}</span>{/if}</div>
  {:else}
    <div class="group-row" data-row={r.id}><span class="words wrap"><span class="sub">{r.label}</span></span></div>
  {/if}
{/snippet}

{#snippet pluginPage()}
  {@const name = card?.plugin.name ?? offer?.name ?? ""}
  {@const about = card?.plugin.about ?? offer?.about}
  {@const fileWord = card?.fromFile ? W.pluginFromFileLine : W.pluginRepoOfficial}
  <header class="head">
    {@render tile(name, card?.icon ?? offer?.icon, true)}
    <span class="words">
      <h1>{name}</h1>
      <span class="sub">{dotted(card?.plugin.version ?? newest?.version, card && W.pluginKinds(card.plugin.points, card.plugin.overrides.length > 0), fileWord)}</span>
    </span>
    {#if card}<button class="glyph" aria-label={W.more} title={W.more} data-plugin-menu onclick={(e) => menu(e, card!)}><Icon name="more-horiz" /></button>{/if}
  </header>
  <div class="intro">
    {#if about}<p>{about}</p>{/if}
    {#if offer}{#each lines(offer) as why}<p class="sub why">{why}</p>{/each}{/if}
    {#if !card && newest && offer && !blocked(offer) && host?.proxied}
      <button class="pill wide" data-install disabled={busy !== ""} onclick={() => getVersion(offer!, newest!)}>{busy === offer.id ? W.pluginInstalling : W.pluginInstall}</button>
    {/if}
    {#if offer?.state === "Update" && newest}
      <button class="pill wide" data-update-to disabled={busy !== ""} onclick={() => getVersion(offer!, newest!)}>{W.pluginUpdateTo(newest.version)}</button>
    {/if}
  </div>
  {#if card}
    {@const i = card}
    <div class="group" data-plugin-page={i.plugin.id}>
      <label class="group-row">
        <span class="words">{W.pluginOn}</span>
        <input type="checkbox" class="switch" aria-label={W.pluginOn} data-plugin-on checked={i.on} onchange={(e) => void host!.setOn(i.plugin.id, e.currentTarget.checked)} />
      </label>
      {#if !i.fromFile}
        <label class="group-row">
          <span class="words">{W.pluginKeepVersion}</span>
          <input type="checkbox" class="switch" aria-label={W.pluginKeepVersion} checked={!!i.held} onchange={(e) => void host!.hold(i.plugin.id, e.currentTarget.checked).then(offered)} />
        </label>
      {/if}
    </div>
    {#each groups(i) as g}
      {#if g.label}<div class="group-label">{g.label}</div>{:else}<div class="gap"></div>{/if}
      <div class="group">
        {#each g.rows as r (r.id)}{@render rowOf(i, r)}{/each}
      </div>
    {/each}
  {/if}
  {#if readme.length > 0}
    <div class="group-label">{W.pluginAbout}</div>
    <div class="readme">
      {#each readme as b}
        {#if b.kind === "Heading"}<h3 class:small={b.level > 2}>{b.text}</h3>
        {:else if b.kind === "Item"}<p class="item"><span>•</span><span>{b.text}</span></p>
        {:else if b.kind === "Code"}<pre>{b.text}</pre>
        {:else}<p>{b.text}</p>{/if}
      {/each}
    </div>
  {/if}
  {#if card && W.pluginMay(card.plugin).length > 0}
    <div class="group-label">{W.pluginMayTitle}</div>
    <div class="group">
      {#each W.pluginMay(card.plugin) as l}<div class="group-row short"><span class="words wrap"><span>{l}</span></span></div>{/each}
    </div>
  {/if}
  {#if offer}
    <div class="group-label">{W.pluginVersions}</div>
    <div class="group">
      {#each offer.versions as v (v.version)}
        <div class="group-row short">
          <span class="words">{v.version}</span>
          {#if v.released}<span class="value">{v.released}</span>{/if}
          {#if v.version !== (card?.plugin.version ?? newest?.version) && !blocked(offer) && host?.proxied}
            <button class="text-action" disabled={busy !== ""} onclick={() => getVersion(offer!, v)}>{W.pluginInstall}</button>
          {/if}
        </div>
      {/each}
      {#if offer.source}
        <a class="group-row short" href={offer.source} target="_blank" rel="noreferrer"><span class="words">{W.pluginSource}</span><span class="value">{host_(offer.source)}</span><Icon name="nav-arrow-right" /></a>
      {/if}
    </div>
  {/if}
{/snippet}

{#if shown === GET}
  {#snippet getHead()}
    <label class="glyph" aria-label={W.pluginsInstallFile} title={W.pluginsInstallFile}>
      <Icon name="folder-plus" />
      <input type="file" accept=".noriplugin" hidden disabled={!host || busy !== ""} onchange={fromFile} />
    </label>
  {/snippet}
  <Page title={W.pluginsGet} back narrow head={getHead}>{@render browser()}</Page>
{:else if shown}
  <Page back narrow ready={!!host && (!!card || !!offer || !!offers || failed)}>{@render pluginPage()}</Page>
{:else if tabbed}
  <div class="tools">{@render tools()}</div>
  {@render list()}
{:else}
  <Page title={W.plugins} back narrow head={tools}>{@render list()}</Page>
{/if}

<style>
  .tools {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 8px;
    margin-bottom: -14px;
  }
  .plugin {
    gap: 12px;
    padding: 0 14px 0 0;
    flex-wrap: wrap;
  }
  .plugin::after {
    left: 70px;
  }
  .plugin:has(.hit:hover) {
    background: var(--hover);
  }
  .hit {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px 0 12px 14px;
    color: inherit;
  }
  .plugin.off > .hit,
  .plugin.off > .value {
    opacity: 0.38;
  }
  .plugin .why {
    flex-basis: 100%;
    margin: -8px 0 8px 70px;
    color: var(--ink);
  }
  .group.off {
    opacity: 0.38;
  }
  .tile {
    width: 44px;
    height: 44px;
    flex: none;
    display: grid;
    place-items: center;
    border-radius: var(--r-card);
    background: var(--veil);
    color: var(--ink);
    font-size: 18px;
    font-weight: 600;
  }
  .tile.big {
    width: 64px;
    height: 64px;
    border-radius: var(--r-tile);
    font-size: 27px;
  }
  /* Its icon in the page's ink. */
  .mark {
    width: 60%;
    height: 60%;
    background: var(--ink);
    mask-size: contain;
    mask-repeat: no-repeat;
    mask-position: center;
  }
  .words .about {
    white-space: normal;
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    line-clamp: 2;
  }
  .tiny {
    font-size: 11.5px;
    line-height: 16px;
  }
  .group-row .wrap > * {
    white-space: normal;
  }
  .short {
    min-height: 44px;
  }
  .quiet {
    margin: 12px 12px;
    font-size: 14px;
    color: var(--ink2);
  }
  .blurb {
    margin: 12px 0 16px;
  }
  .wide {
    display: flex;
    width: 100%;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 14px;
    margin-top: 6px;
  }
  .head .words {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .head h1 {
    font-size: 26px;
    line-height: 32px;
  }
  .intro {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin: 12px 0 18px;
  }
  .intro p {
    margin: 0;
  }
  .why {
    color: var(--ink);
  }
  .gap {
    height: var(--gap-section);
  }
  .find {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 16px;
  }
  .level {
    flex-direction: column;
    align-items: stretch;
    gap: 6px;
    padding-top: 10px;
    padding-bottom: 10px;
  }
  .level-line {
    color: var(--ink);
  }
  .level .range {
    width: 100%;
  }
  .text {
    flex-direction: column;
    align-items: stretch;
    gap: 4px;
    padding-top: 10px;
    padding-bottom: 10px;
  }
  .field {
    padding: 8px 10px;
    border-radius: var(--r-cover);
    border: var(--hairline) solid var(--hair);
    background: none;
    color: var(--ink);
    font: inherit;
  }
  .field:focus {
    outline: 0;
    border-color: var(--accent);
  }
  .readme {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 0 12px;
  }
  .readme p,
  .readme h3,
  .readme pre {
    margin: 0;
  }
  .readme h3 {
    margin-top: 6px;
    font-size: 16px;
    font-weight: 600;
  }
  .readme h3.small {
    font-size: 14px;
  }
  .item {
    display: flex;
    gap: 8px;
  }
  pre {
    padding: 10px;
    overflow-x: auto;
    border-radius: var(--r-card);
    background: var(--surface);
    font-size: 13px;
  }
</style>
