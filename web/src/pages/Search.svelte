<script lang="ts">
  // The search's results page, as the core lays it out (`search_layout`): the kind chips (All, then the kinds
  // found) and, where providers' results are mixed in, the scope chips; on All the top result with the best
  // songs beside it (under it on a phone), then a row of albums, artists and playlists; with a kind picked,
  // all of it in place. A click on a song plays that song alone (a search's results are not an album).
  import { facts, read } from "../rpc";
  import { reading } from "../state/library.svelte";
  import { player } from "../state/player.svelte";
  import { songMenu } from "../state/menus";
  import { href, route, searchFor } from "../state/route.svelte";
  import { forgetSearches, narrow, pick, remember, search, typed, type Item, type Kind, type View } from "../state/search.svelte";
  import { pointer, ui } from "../state/ui.svelte";
  import type { Decade, Genre, Song } from "../state/types";
  import { W } from "../words";
  import Cards from "../ui/Cards.svelte";
  import Cover from "../ui/Cover.svelte";
  import Icon from "../ui/Icon.svelte";
  import Page from "../ui/Page.svelte";
  import SectionTitle from "../ui/SectionTitle.svelte";
  import SongList from "../ui/SongList.svelte";

  let field = $state<HTMLInputElement>();
  // The route holds the query: typed here, in the top bar, or reached by a link or Back.
  const page = reading("search", () => typed(route.arg));
  const v = $derived(search.view && search.view.state !== "blank" ? search.view : null);
  $effect(() => {
    if (v && v.state !== "asking") facts.searched = { query: v.query, artists: v.artists.length, albums: v.albums.length, songs: v.songs.length };
  });
  const busy = $derived(v?.state === "asking");
  // Before a search, ways in that need no words, as Android's: the library's biggest genres and its decades
  // (the core's `genre_tiles` and `browse_decades`).
  let browse = $state.raw<{ genres: Genre[]; decades: Decade[] }>();
  $effect(() => {
    void route.visit;
    void read<{ genres: Genre[]; decades: Decade[] }>({ kind: "browse" }).then(
      (b) => (browse = b),
      () => {},
    );
  });
  const items = (view: View, kind: Kind) => view.layout.sections.find((s) => s.kind === kind)?.items ?? [];
  $effect(() => {
    if (!ui.wide) field?.focus();
  });

  /** A song found, played alone. */
  function playOne(s: Song) {
    remember();
    void player.play([s], 0, false, "Search", v?.query ?? "");
  }
  const kindNames: Record<Kind, string> = { songs: W.songs, albums: W.albums, artists: W.artists, playlists: W.playlists };

  /** The top result's name, its line, its cover and where it leads. */
  function top(view: View, t: Item) {
    switch (t.kind) {
      case "songs": {
        const s = view.songs[t.at];
        return { name: s.title, line: s.artist, cover: s.cover, remote: !!s.isExternal, href: undefined, round: false };
      }
      case "albums": {
        const a = view.albums[t.at];
        return { name: a.name, line: a.subtitle ?? a.artist, cover: a.cover, remote: !!a.isExternal, href: href("album", a.id), round: false };
      }
      case "artists": {
        const a = view.artists[t.at];
        return { name: a.name, line: a.albumCount ? W.albumCount(a.albumCount) : "", cover: a.cover, remote: !!a.isExternal, href: href("artist", a.id), round: true };
      }
      case "playlists": {
        const p = view.playlists[t.at];
        return { name: p.name, line: p.songCount ? W.songCount(p.songCount) : (p.comment ?? ""), cover: p.cover, remote: !!p.remote, href: href("playlist", p.id), round: false };
      }
    }
  }
  /** The top result played: a song alone, else what it is from its start. */
  function playTop(view: View, t: Item) {
    remember();
    if (t.kind === "songs") return playOne(view.songs[t.at]);
    const id = t.kind === "albums" ? view.albums[t.at].id : t.kind === "artists" ? view.artists[t.at].id : view.playlists[t.at].id;
    void player.playWhole(t.kind === "albums" ? "album" : t.kind === "artists" ? "artist" : "playlist", id, false);
  }
</script>

<!-- Before a search, a narrow page: in the top bar's column, as the desktop's. -->
<Page title={W.search} narrow={!route.arg.trim()} error={page.error}>
  {#if !ui.wide}
    <label class="field search-field box"><Icon name="search" /><input bind:this={field} type="search" placeholder={W.searchHint} aria-label={W.search} value={route.arg} oninput={(e) => searchFor(e.currentTarget.value)} onkeydown={(e) => e.key === "Enter" && (remember(), e.currentTarget.blur())} /></label>
  {/if}
  <div class="busy" class:on={busy} role="progressbar" aria-hidden={!busy} aria-label={W.loading}></div>
  {#if !route.arg.trim()}
    {#if search.recent.length}
      <div class="recent-head"><SectionTitle title={W.recentSearches} /><button class="text-action" onclick={() => void forgetSearches()}>{W.clearSearches}</button></div>
      <ul class="recent">
        {#each search.recent as q (q)}<li><button onclick={() => searchFor(q)}><Icon name="clock-rotate-right" /><span class="clip">{q}</span></button></li>{/each}
      </ul>
    {/if}
    {#if browse?.genres.length}
      <SectionTitle title={W.librarySections.Genres} href={href("genres")} />
      <div class="genres">
        {#each browse.genres as g (g.name)}
          <a class="genre" href={href("genre", g.name)}>
            <span class="art"><Cover src={g.cover} radius="0" /></span>
            <span class="words"><span class="name clip">{g.name}</span><span class="sub clip">{W.songCount(g.songCount)}</span></span>
          </a>
        {/each}
      </div>
    {/if}
    {#if browse?.decades.length}
      <SectionTitle title={W.librarySections.Decades} href={href("decades")} />
      <div class="chips decades">
        {#each browse.decades as d (d.start)}<a class="chip on" href={href("decade", String(d.start))}>{W.decade(d.start)}</a>{/each}
      </div>
    {/if}
  {:else if v}
    {#if v.kinds.length}
      <div class="chips" id="search-kinds" role="tablist" aria-label={W.search}>
        <button class="chip" class:on={v.kind === null} role="tab" aria-selected={v.kind === null} data-kind="all" onclick={() => void pick(null)}>{W.searchAll}</button>
        {#each v.kinds as k (k)}<button class="chip" class:on={v.kind === k} role="tab" aria-selected={v.kind === k} data-kind={k} onclick={() => void pick(k)}>{kindNames[k]}</button>{/each}
        {#if v.scopes.length}
          <span class="gap" aria-hidden="true"></span>
          {#each v.scopes as sc (sc)}<button class="chip" class:on={v.scope === sc} aria-pressed={v.scope === sc} data-scope={sc} onclick={() => void narrow(sc)}>{W.searchScope[sc]}</button>{/each}
        {/if}
      </div>
    {/if}
    {#if v.fallback !== null}<p class="note">{W.searchFallback(v.fallback)}</p>{/if}
    {#if v.state === "nothing"}
      <p class="empty"><Icon name="search" />{W.noResults}</p>
    {:else if v.kind === null}
      {@const songs = items(v, "songs")}
      {#if v.layout.top || songs.length}
        <div class="best" class:alone={!v.layout.top}>
          {#if v.layout.top}
            {@const t = v.layout.top}
            {@const c = top(v, t)}
            <section class="top-col">
              <SectionTitle title={W.topResult} />
              <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
              <div class="top" onclick={() => (c.href ? (remember(), (location.hash = c.href)) : playTop(v, t))} id="search-top">
                <div class="art"><Cover src={c.cover} radius="0" fallback={t.kind === "artists" ? "user" : "music-note"} /></div>
                <div class="words">
                  <span class="label">{W.topKind[t.kind]}</span>
                  <span class="title">{c.name}</span>
                  <span class="line clip">{#if c.remote}<Icon name="cloud-download" />{/if}{c.line}</span>
                  <button
                    class="play"
                    onclick={(e) => {
                      e.stopPropagation();
                      playTop(v, t);
                    }}><Icon name="play-solid" />{W.play}</button
                  >
                </div>
              </div>
            </section>
          {/if}
          {#if songs.length}
            <section class="songs-col">
              <SectionTitle title={W.songs} />
              <ol class="few">
                {#each songs as i (v.songs[i].id)}
                  {@const s = v.songs[i]}
                  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
                  <li class:now={s.id === player.song?.id} oncontextmenu={(e) => (e.preventDefault(), void songMenu(s, pointer(e), { play: () => playOne(s) }))}>
                    <button onclick={() => playOne(s)}>
                      <span class="art"><Cover src={s.cover} radius="4px" /><Icon name="play-solid" /></span>
                      <span class="words"><span class="title clip">{s.title}</span><span class="sub clip">{#if s.isExternal}<Icon name="cloud-download" />{/if}{s.artist}</span></span>
                      <span class="len">{W.time(s.duration * 1000)}</span>
                    </button>
                  </li>
                {/each}
              </ol>
            </section>
          {/if}
        </div>
      {/if}
      {#if items(v, "albums").length}<SectionTitle title={W.albums} /><Cards albums={items(v, "albums").map((i) => v.albums[i])} row />{/if}
      {#if items(v, "artists").length}<SectionTitle title={W.artists} /><div class="artists"><Cards artists={items(v, "artists").map((i) => v.artists[i])} row /></div>{/if}
      {#if items(v, "playlists").length}<SectionTitle title={W.playlists} /><Cards playlists={items(v, "playlists").map((i) => v.playlists[i])} row />{/if}
    {:else if v.kind === "songs"}
      {#key v.query}<SongList songs={v.songs} kind="Search" id={v.query} one />{/key}
    {:else if v.kind === "albums"}
      <Cards albums={v.albums} />
    {:else if v.kind === "artists"}
      <Cards artists={v.artists} />
    {:else}
      <Cards playlists={v.playlists} />
    {/if}
  {/if}
</Page>

<style>
  .box {
    margin: 8px 0;
  }
  /* Always there, so the results never move as it shows; a sweep while the server answers. */
  .busy {
    height: 2px;
    overflow: hidden;
    position: relative;
  }
  .busy.on {
    background: var(--hair);
  }
  .busy.on::after {
    content: "";
    position: absolute;
    inset: 0 auto 0 0;
    width: 30%;
    background: var(--ink);
    animation: sweep 1.1s ease-in-out infinite;
  }
  @keyframes sweep {
    from {
      transform: translateX(-100%);
    }
    to {
      transform: translateX(340%);
    }
  }
  .chips {
    margin-bottom: 8px;
  }
  .chips button {
    cursor: pointer;
  }
  .chips .chip:not(.on):hover {
    color: var(--ink);
  }
  .gap {
    flex: none;
    width: 1px;
    margin: 8px 4px;
    background: var(--hair);
  }
  .note {
    margin: 0 0 12px;
    color: var(--ink2);
    font-size: 13px;
  }
  /* The top result and the best songs side by side on a wide page, one over the other on a narrow one. */
  .best {
    display: grid;
    grid-template-columns: minmax(0, 45%) minmax(0, 1fr);
    gap: 0 32px;
    max-width: 100%;
  }
  .best.alone {
    grid-template-columns: minmax(0, 1fr);
  }
  @media (max-width: 899px) {
    .best {
      grid-template-columns: minmax(0, 1fr);
    }
  }
  .top-col {
    max-width: 520px;
  }
  /* As Home's Continue card: raised paper, the cover running into it, what it is, its name and Play. */
  .top {
    display: flex;
    height: 224px;
    border-radius: var(--r-tile);
    overflow: hidden;
    background: var(--surface);
    border: var(--hairline) solid var(--hair);
    cursor: pointer;
  }
  .top .art {
    position: relative;
    width: 224px;
    flex: none;
  }
  .top .art::after {
    content: "";
    position: absolute;
    inset: 0;
    background: linear-gradient(to right, transparent 55%, var(--surface));
  }
  .top .art :global(.cover) {
    height: 100%;
  }
  .top .words {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    padding: 18px 18px 16px 6px;
  }
  .label {
    font-size: 11.5px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--ink) 65%, transparent);
  }
  .top .title {
    margin-top: 4px;
    font: 600 26px/30px var(--display);
    letter-spacing: -0.3px;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .line {
    margin-top: 2px;
    font-size: 13px;
    color: color-mix(in srgb, var(--ink) 70%, transparent);
  }
  .line :global(.icon),
  .sub :global(.icon) {
    width: 13px;
    height: 13px;
    margin-right: 4px;
    vertical-align: -2px;
  }
  .play {
    margin-top: auto;
    align-self: flex-start;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 38px;
    padding: 0 16px 0 12px;
    border-radius: 19px;
    background: var(--ink);
    color: var(--surface);
    font-size: 14px;
    font-weight: 600;
  }
  .play :global(.icon) {
    width: 18px;
    height: 18px;
  }
  @media (max-width: 719px) {
    .top {
      height: 178px;
    }
    .top .art {
      width: 178px;
    }
    .top .title {
      font-size: 21px;
      line-height: 24px;
    }
    .songs-col {
      margin-top: 8px;
    }
  }
  /* The best songs as Home's quick picks: the cover, the title and artist, the time; the row raised under the
     pointer, a play glyph over the cover. */
  .few {
    list-style: none;
    margin: 0 -6px;
    padding: 0;
  }
  .few button {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 6px;
    border-radius: 10px;
    text-align: left;
  }
  .few button:hover {
    background: var(--hover);
  }
  .few .art {
    position: relative;
    width: 44px;
    flex: none;
  }
  .few .art :global(.icon) {
    position: absolute;
    inset: 0;
    margin: auto;
    width: 16px;
    height: 16px;
    color: var(--ink);
    opacity: 0;
  }
  .few button:hover .art :global(.icon) {
    opacity: 1;
  }
  .few button:hover .art :global(.cover) {
    opacity: 0.45;
  }
  .few .words {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .few .title {
    font-size: 14px;
    line-height: 19px;
    font-weight: 500;
  }
  .few .now .title {
    color: var(--accent);
  }
  .few .sub {
    font-size: 12.5px;
    line-height: 16px;
    color: var(--ink2);
  }
  .len {
    color: var(--ink2);
    font-size: 13px;
    font-variant-numeric: tabular-nums;
  }
  /* A genre as Android's tile: its name in Literata on the surface, a cover of its songs at the end running
     into the paper; as many columns as the page holds. */
  .genres {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: 12px;
  }
  .genre {
    position: relative;
    display: flex;
    align-items: center;
    height: 72px;
    border-radius: var(--r-tile);
    overflow: hidden;
    background: var(--surface);
    border: var(--hairline) solid var(--hair);
  }
  .genre:hover {
    background: color-mix(in srgb, var(--ink) 5%, var(--surface));
  }
  .genre .art {
    position: absolute;
    top: 0;
    right: 0;
    width: 72px;
    height: 72px;
    -webkit-mask-image: linear-gradient(to right, transparent, #000 75%);
    mask-image: linear-gradient(to right, transparent, #000 75%);
  }
  .genre .art :global(.cover) {
    height: 100%;
  }
  .genre .words {
    position: relative;
    min-width: 0;
    display: flex;
    flex-direction: column;
    padding: 0 36px 0 14px;
  }
  .genre .name {
    font: 600 17px/22px var(--display);
  }
  .decades {
    flex-wrap: wrap;
  }
  .recent-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .recent {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .recent button {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 9px 0;
    font-size: 16px;
    text-align: left;
  }
  .recent :global(.icon) {
    width: 20px;
    height: 20px;
    color: var(--ink2);
    flex: none;
  }
</style>
