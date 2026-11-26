<script lang="ts">
  // The listening stats, as Android's (from History): whose plays and how far back, the plays in large type
  // with what else they count on plates, the listening day as 24 bars and when it peaks, then the top lists.
  // A wide window sets the plates beside the plays and the top lists two across (the core's `stats_page`).
  import { untrack } from "svelte";
  import { ask } from "../rpc";
  import { reading } from "../state/library.svelte";
  import { W } from "../words";
  import Cover from "../ui/Cover.svelte";
  import Page from "../ui/Page.svelte";
  import SectionTitle from "../ui/SectionTitle.svelte";

  type Entry = { name: string; artist?: string; plays: number; listenedMs: number; cover?: string };
  type Stats = {
    source: string;
    server: string | null;
    plays: number;
    listenedMs: number;
    tiles: { kind: string; value: number }[];
    hours: number[];
    busiestHour: number | null;
    busiestWeekday: number | null;
    songs: Entry[];
    artists: Entry[];
    albums: Entry[];
    genres: Entry[];
  };
  type Choices = { periods: string[]; sources: string[]; opened: string };

  const offered = ask<string>("stats_choices").then((c) => JSON.parse(c) as Choices);
  let choices = $state<Choices>();
  let period = $state<string>();
  let source = $state<string>();
  /** How the server account's plays stood when first asked for; they are asked for once a visit. */
  let server = $state<string | null>(null);
  const shownPeriod = $derived(period ?? choices?.opened);
  const shownSource = $derived(source ?? choices?.sources[0]);

  const page = reading("stats", async () => {
    const [p, s] = [period, source];
    const c = await offered;
    choices = c;
    const from = s ?? c.sources[0];
    const sync = from === "ServerAccount" && untrack(() => server) === null;
    const d = JSON.parse(await ask<string>("stats", p ?? c.opened, from, sync)) as Stats;
    if (d.server) server = d.server;
    return d;
  });

  const fromServer = $derived(page.data?.source === "ServerAccount");
  /** The top lists with something in them: a line under each name, an artist's cover round. */
  const lists = $derived.by(() => {
    const d = page.data;
    if (!d) return [];
    const timed = (e: Entry) => (fromServer ? "" : W.time(e.listenedMs));
    return [
      { title: W.topSongs, round: false, rows: d.songs.map((e) => ({ ...e, sub: e.artist ?? "" })) },
      { title: W.topArtists, round: true, rows: d.artists.map((e) => ({ ...e, sub: timed(e) })) },
      { title: W.topAlbums, round: false, rows: d.albums.map((e) => ({ ...e, sub: "" })) },
      { title: W.topGenres, round: false, rows: d.genres.map((e) => ({ ...e, sub: "" })) },
    ].filter((l) => l.rows.length > 0);
  });
  const TICKS = [0, 6, 12, 18, 23];
</script>

<Page title={W.listening} back ready={!!page.data} error={page.error}>
  {#if choices}
    <div class="choices-bar">
      {#each choices.sources as s (s)}
        <button class="chip" class:on={s === shownSource} aria-pressed={s === shownSource} onclick={() => (source = s)}>{W.statsSources[s]}</button>
      {/each}
      <span class="rule" aria-hidden="true"></span>
      {#each choices.periods as p (p)}
        <button class="chip" class:on={p === shownPeriod} aria-pressed={p === shownPeriod} onclick={() => (period = p)}>{W.statsPeriods[p]}</button>
      {/each}
    </div>
  {/if}
  {#if page.data}
    {@const d = page.data}
    <section class="hero">
      <div class="headline">
        <p class="plays">{d.plays}</p>
        <p class="what">{W.statsHeadline(fromServer, d.listenedMs)}</p>
        {#if fromServer}<p class="note">{W.statsServerNote(server)}</p>{/if}
      </div>
      <ul class="tiles" class:five={d.tiles.length === 5}>
        {#each d.tiles as t (t.kind)}<li><b>{t.value}</b><span>{W.statKinds[t.kind]}</span></li>{/each}
      </ul>
    </section>
    {#if d.busiestHour !== null}
      <section class="day">
        <SectionTitle title={W.whenYouListen} />
        <div class="bars" aria-hidden="true">
          {#each d.hours as share, h (h)}<span class="bar"><span style:height="{share * 100}%"></span></span>{/each}
        </div>
        <div class="ticks" aria-hidden="true">
          {#each TICKS as h (h)}<span style:grid-column={h + 1}>{String(h).padStart(2, "0")}</span>{/each}
        </div>
        <p class="habit">{W.statsHabit(d.busiestHour, d.busiestWeekday)}</p>
      </section>
    {/if}
    <div class="tops">
      {#each lists as l (l.title)}
        <section>
          <SectionTitle title={l.title} />
          <ol>
            {#each l.rows as r, i (i)}
              <li>
                <span class="rank">{i + 1}</span>
                <span class="art"><Cover src={r.cover} round={l.round} /></span>
                <span class="text"><span class="name">{r.name}</span>{#if r.sub}<span class="sub">{r.sub}</span>{/if}</span>
                <span class="count">{r.plays}</span>
              </li>
            {/each}
          </ol>
        </section>
      {/each}
    </div>
  {/if}
</Page>

<style>
  .choices-bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    margin: -6px 0 28px;
  }
  .chip {
    border-color: transparent;
  }
  .chip.on {
    border-color: var(--hair);
  }
  .rule {
    width: 1px;
    height: 20px;
    margin: 0 8px;
    background: var(--line);
  }
  .hero {
    display: grid;
    gap: 20px;
    margin-bottom: 40px;
  }
  .plays {
    margin: 0;
    font-size: 64px;
    line-height: 1;
    font-weight: 700;
    letter-spacing: -1.5px;
    font-variant-numeric: tabular-nums;
  }
  .what {
    margin: 8px 0 0;
    color: var(--ink2);
    font-size: 17px;
    font-weight: 500;
  }
  .note {
    margin: 8px 0 0;
    color: var(--ink2);
    font-size: 13px;
    max-width: 46ch;
  }
  .tiles {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(6, 1fr);
    gap: 10px;
  }
  .tiles li {
    grid-column: span 2;
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 2px;
    min-height: 84px;
    padding: 0 16px;
    border-radius: var(--r-tile);
    background: var(--veil);
    min-width: 0;
  }
  /* Without skips, the second row's two share it. */
  .five li:nth-child(n + 4) {
    grid-column: span 3;
  }
  .tiles b {
    font-size: 26px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
  .tiles span {
    color: var(--ink2);
    font-size: 13px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .day {
    margin-bottom: 40px;
  }
  .bars {
    display: grid;
    grid-template-columns: repeat(24, 1fr);
    height: 104px;
    margin-top: 12px;
  }
  .bar {
    justify-self: center;
    width: min(62%, 24px);
    display: flex;
    align-items: flex-end;
    border-radius: 999px;
    background: color-mix(in srgb, var(--ink) 12%, transparent);
    overflow: hidden;
  }
  .bar span {
    width: 100%;
    border-radius: 999px;
    background: var(--ink);
  }
  .ticks {
    display: grid;
    grid-template-columns: repeat(24, 1fr);
    margin-top: 6px;
    color: var(--ink2);
    font-size: 11px;
    font-weight: 600;
  }
  .ticks span {
    text-align: center;
  }
  .habit {
    margin: 6px 0 0;
    color: var(--ink2);
    font-size: 14px;
  }
  .tops {
    display: grid;
    gap: 32px;
  }
  ol {
    list-style: none;
    margin: 6px 0 0;
    padding: 0;
  }
  ol li {
    display: grid;
    grid-template-columns: 22px 44px 1fr auto;
    align-items: center;
    gap: 12px;
    height: 56px;
  }
  .rank {
    color: var(--ink2);
    font-size: 14px;
    font-weight: 600;
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .art {
    width: 44px;
  }
  .text {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .name,
  .sub {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .name {
    font-size: 14px;
    font-weight: 500;
  }
  .sub,
  .count {
    color: var(--ink2);
    font-size: 13px;
  }
  .count {
    font-variant-numeric: tabular-nums;
  }
  @media (min-width: 960px) {
    .hero {
      grid-template-columns: minmax(240px, 1fr) 2fr;
      gap: 48px;
      align-items: start;
      margin-bottom: 44px;
    }
    .bars {
      height: 150px;
    }
    .tops {
      grid-template-columns: 1fr 1fr;
      column-gap: 48px;
    }
  }
</style>
