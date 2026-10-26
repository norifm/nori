// What the core's pages and the player read as (crates/web: pages.rs, play.rs).

export type Song = {
  id: string;
  title: string;
  artist: string;
  album: string;
  albumId?: string;
  artistId?: string;
  duration: number;
  track?: number;
  cover?: string;
  suffix?: string;
  starred?: boolean;
  /** Its second line, as the core words it (the explicit mark and the artist). */
  line?: string;
};
/** `subtitle`: a card's line under its name, as the core words it. */
export type Album = { id: string; name: string; artist: string; artistId?: string; year: number; cover?: string; songCount: number; duration: number; genre?: string; starred?: boolean; subtitle?: string };
/** What a card plays whole. */
export type Whole = "album" | "playlist" | "artist" | "mix";
/** A mix on Home's "For you" (the core's `mix_cards`): `name` the core's `MixName`, up to four covers, and
 * its card's paper and ink (`mix_card_colours`). */
export type MixTile = { id: string; name: string; favourites: boolean; covers: string[]; page: string; ink: string };
/** A mix's page (the core's `mix_page`): the songs as they play. */
export type MixPage = { id: string; name: string; favourites: boolean; seconds: number; covers: string[]; songs: Song[] };
export type Artist = { id: string; name: string; cover?: string; albumCount?: number | null; starred?: boolean };
export type Playlist = { id: string; name: string; songCount: number; duration: number; cover?: string; comment?: string; owner?: string; /** The account may change it (the core's `playlist_editable`). */ editable?: boolean };
/** As the server names it: the name is `value`. */
export type Genre = { value: string; songCount: number; albumCount: number };

/** A song of the queue the player shows, in play order: its list `index`, who asked for it in a jam. */
export type QueueSong = Partial<Song> & { index: number; by?: string };

export type Now = {
  state: "Idle" | "Loading" | "Playing" | "Paused" | "Stopped" | string;
  /** The play button shows playing. */
  playing: boolean;
  index: number | null;
  id: string | null;
  positionMs: number;
  pace: number;
  mixing: boolean;
  buffering: boolean;
  song: Song | null;
  queue: QueueSong[];
  /** The place in `queue` of the song heard; -1 for none. */
  current: number;
  reorderable: boolean;
  shuffle: boolean;
  repeat: number;
  /** The volume of the device playing, 0 to 1. */
  volume: number;
  /** The device playing elsewhere, while it is another one. */
  device: string | null;
  /** That device's kind (`Phone`, `Desktop`, `Terminal`, `Guest`). */
  deviceKind: string | null;
  jam: boolean;
  /** A jam guest's picks are asked of the host; its lyric taps reach nothing. */
  asks: boolean;
  underruns: number;
  transition?: { kind: string; into: string; reason: string } | null;
};

// Field names as nori-model's serde has them.
export type LyricLine = { start_ms: number; end_ms: number; text: string; translation?: string | null; background?: boolean; voice?: number };
export type Lyrics = { synced: boolean; word_timed: boolean; lines: LyricLine[] };
export type Timing = { kind: "NotChecked" | "DoesNotFit" | "Unsure" | "Automatic" | "Yours"; ms: number; automatic?: number | null };
/** The lyrics shown for a song, where they came from and how they are timed. */
export type Shown = { song: string; lyrics: Lyrics; origin: string; timing: Timing };

export type Jam = {
  start: "Offered" | "Starting" | "Hosting" | "Unsupported" | "Unavailable";
  guest: boolean;
  hosting: boolean;
  link: string | null;
  host: string | null;
  you: string | null;
  members: { id: string; name: string; role: string }[];
  pending: { request: number; from: string; fromId: string; provider: boolean; song: Song }[];
  along: boolean;
  remoteSongs: boolean;
  listening: string | null;
  asks: boolean;
};

export type Devices = { on: boolean; me?: string; active: string | null; list: { id: string; name: string; kind: string; playing: boolean; nearby: boolean }[] };

export type SettingRow = {
  item: { kind: "setting"; name: string; type: "Switch" | "Choice" | "Level" | "Text" | "Colour"; options: string[]; min: number; max: number; value: string; chips: boolean } | { kind: "note"; note: string } | { kind: "link"; to: string };
  shown: boolean;
  enabled: boolean;
  depth: number;
};
export type SettingsSection = { id: string; rows: SettingRow[] };

export type MenuAction = { action: string; more: boolean; enabled: boolean; on?: boolean; id?: string; name?: string; named?: boolean };

/** A band as the equalizer page draws it (crates/web/src/eq.rs): `x` across the curve (0..1), `db` its
 * point's height, the rest its editor's. */
export type EqBand = { x: number; db: number; gain: number; label: string; value: string; usesGain: boolean; kind: number; channel: number; q: number; freq: string; shape: string };
export type Equalizer = {
  on: boolean;
  graphic: boolean;
  count: number;
  layouts: number[];
  reach: number;
  curve: number[];
  each: number[][];
  bands: EqBand[];
  presets: string[];
  preamp: number;
  auto: boolean;
  balance: number;
  mono: boolean;
  limiter: boolean;
  ceiling: number;
  ranges: { gain: [number, number]; preamp: [number, number]; balance: [number, number]; limiter: [number, number]; q: [number, number] };
  bypass: string | null;
};
