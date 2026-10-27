<script lang="ts">
  // Every artist, narrowed by name in the field over them (what it keeps is the core's, `filter_rows`). On
  // a wide window as the desktop's: the names down the left, the one chosen (the first, until another is)
  // on the right with Play, Shuffle and its releases, a double click or its name opening its page. On a
  // phone as Android's: a row each, its picture round, opening its page.
  import { ask, read } from "../rpc";
  import { reading } from "../state/library.svelte";
  import { player } from "../state/player.svelte";
  import { go, href } from "../state/route.svelte";
  import { ui } from "../state/ui.svelte";
  import type { Album, Artist } from "../state/types";
  import { W } from "../words";
  import Cards from "../ui/Cards.svelte";
  import Cover from "../ui/Cover.svelte";
  import Icon from "../ui/Icon.svelte";
  import Page from "../ui/Page.svelte";

  const page = reading("artists", () => read<{ artists: Artist[] }>({ kind: "artists" }).then((p) => p.artists ?? []));
  let filter = $state("");
  let shown = $state.raw<Artist[]>([]);
  $effect(() => {
    const all = page.data ?? [];
    const f = filter;
    if (!f.trim()) return void (shown = all);
    let gone = false;
    void ask<string>("filter_rows", JSON.stringify(all.map((a) => [a.name])), f).then((r) => !gone && (shown = (JSON.parse(r) as number[]).map((i) => all[i])));
    return () => {
      gone = true;
    };
  });

  /** Where the panes start: under the page's head, wherever its back button and title put it. */
  let holder = $state<HTMLElement>();
  let top = $state(0);
  $effect(() => {
    if (!holder) return;
    const at = holder;
    const measure = () => (top = at.getBoundingClientRect().top + (document.getElementById("main")?.scrollTop ?? 0));
    measure();
    addEventListener("resize", measure);
    return () => removeEventListener("resize", measure);
  });

  let picked = $state<string | null>(null);
  /** The artist on the right: the one picked while it is listed, else the first listed. */
  const chosen = $derived(shown.find((a) => a.id === picked) ?? shown[0]);
  type Group = { kind: string; tag: string; albums: number[] };
  let detail = $state.raw<{ id: string; albums: Album[]; groups: Group[] } | null>(null);
  $effect(() => {
    const id = ui.wide ? chosen?.id : undefined;
    if (!id) return;
    let gone = false;
    void read<{ albums: Album[]; groups: Group[] }>({ kind: "artist", id }).then((d) => !gone && (detail = { id, albums: d.albums ?? [], groups: d.groups ?? [] }));
    return () => {
      gone = true;
    };
  });
  const here = $derived(detail && detail.id === chosen?.id ? detail : null);
  const shelves = $derived((here?.groups ?? []).map((g) => ({ name: W.shelf(g.kind, g.tag), albums: g.albums.map((i) => here!.albums[i]) })));
  const caption = $derived.by(() => {
    const albums = here?.albums ?? [];
    if (!albums.length) return "";
    const n = albums.reduce((s, x) => s + (x.songCount || 0), 0);
    return [W.releases(albums.length), n ? W.songCount(n) : ""].filter(Boolean).join(" · ");
  });
</script>

<Page title={W.artists} back ready={!!page.data} error={page.error}>
  {#snippet head()}<label class="field search-field filter"><Icon name="search" /><input type="search" placeholder={W.filterArtists} aria-label={W.filterArtists} bind:value={filter} /></label>{/snippet}
  {#if !shown.length}
    <p class="empty"><Icon name="user" />{W.empty}</p>
  {:else if ui.wide}
    <div bind:this={holder}></div>
    <div class="panes" style:top="{top}px">
      <ul class="names" role="listbox" aria-label={W.artists}>
        {#each shown as a (a.id)}
          <li>
            <button class="name" class:chosen={a.id === chosen?.id} role="option" aria-selected={a.id === chosen?.id} onclick={() => (picked = a.id)} ondblclick={() => go("artist", a.id)}>
              <span class="art"><Cover src={a.cover} round fallback="user" /></span><span class="clip">{a.name}</span>
            </button>
          </li>
        {/each}
      </ul>
      <section class="chosen-pane">
        {#if chosen}
          <div class="title">
            <a class="clip" href={href("artist", chosen.id)}>{chosen.name}</a>
            <button class="plate" aria-label={W.play} title={W.play} onclick={() => void player.playWhole("artist", chosen.id, false)}><Icon name="play-solid" /></button>
            <button class="plate" aria-label={W.shuffle} title={W.shuffle} onclick={() => void player.playWhole("artist", chosen.id, true)}><Icon name="shuffle" /></button>
          </div>
          <div class="caption">{caption}</div>
          {#each shelves as s, n (n)}
            <h2 class="shelf">{s.name}</h2>
            <Cards albums={s.albums} row />
          {/each}
        {/if}
      </section>
    </div>
  {:else}
    <ul class="rows">
      {#each shown as a (a.id)}
        <li>
          <a class="row" href={href("artist", a.id)}>
            <span class="art big"><Cover src={a.cover} round fallback="user" /></span>
            <span class="words"><span class="clip">{a.name}</span>{#if a.albumCount}<span class="sub">{W.albumCount(a.albumCount)}</span>{/if}</span>
          </a>
        </li>
      {/each}
    </ul>
  {/if}
</Page>

<style>
  .filter {
    width: min(320px, 100%);
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .empty {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--ink2);
  }
  /* Under the page's head, to the window's foot; each side scrolls on its own, under the player. */
  .panes {
    position: fixed;
    bottom: 0;
    left: 0;
    right: 0;
    display: grid;
    grid-template-columns: 300px 1fr;
  }
  .names,
  .chosen-pane {
    overflow-y: auto;
    padding-bottom: calc(var(--bar-h) + 12px);
  }
  .names {
    padding: 0 12px calc(var(--bar-h) + 12px);
    border-right: 1px solid var(--line);
  }
  .name {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    height: 52px;
    padding: 0 10px;
    border-radius: 9px;
    font-size: 13px;
    text-align: left;
  }
  .name:hover {
    background: var(--hover);
  }
  /* The one chosen is raised, never filled with the accent. */
  .name.chosen {
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--hair);
  }
  .art {
    width: 38px;
    flex: none;
  }
  .art.big {
    width: 48px;
  }
  .chosen-pane {
    padding: 16px 30px calc(var(--bar-h) + 12px);
    min-width: 0;
  }
  .title {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .title a {
    flex: 1;
    min-width: 0;
    font-size: 28px;
    line-height: 36px;
    font-weight: 700;
  }
  .title a:hover {
    text-decoration: underline;
  }
  .plate {
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    border-radius: 50%;
    background: var(--ink);
    color: var(--page);
    flex: none;
  }
  .plate :global(.icon) {
    width: 15px;
    height: 15px;
  }
  .caption {
    margin: 4px 0 12px;
    padding-bottom: 12px;
    border-bottom: 1px solid var(--line);
    font-size: 12px;
    font-weight: 600;
    color: var(--ink2);
    min-height: 15px;
  }
  .shelf {
    font-size: 17px;
    margin: 18px 0 10px;
  }
  .rows .row {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 7px 0;
    position: relative;
  }
  .rows li + li .row::before {
    content: "";
    position: absolute;
    top: 0;
    left: 60px;
    right: 0;
    height: var(--hairline);
    background: var(--hair);
  }
  .words {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    font-size: 16px;
  }
  .sub {
    font-size: 13px;
    color: var(--ink2);
  }
</style>
