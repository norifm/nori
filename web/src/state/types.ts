// What the core's pages and the player read as (crates/web: pages.rs, play.rs).

export type Song = {
  id: string;
  /** What tells it apart from another source's song of the same id, in the queue and the player. */
  key: string;
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
  artists?: { id: string; name: string }[];
  discNumber?: number;
  year?: number;
  genre?: string;
  explicitStatus?: "" | "clean" | "explicit";
  replayGain?: { trackGain?: number; albumGain?: number; trackPeak?: number; albumPeak?: number };
  bpm?: number;
  userRating?: number;
  /** A provider's, not in the library yet. */
  isExternal?: boolean;
  /** Which merged source of the profile it comes from; omitted for the profile's own. */
  source?: string | null;
  sourceMarked?: boolean;
};
/** `subtitle`: a card's line under its name, as the core words it. */
export type Album = { id: string; name: string; artist: string; artistId?: string; year: number; cover?: string; songCount: number; duration: number; genre?: string; starred?: boolean; subtitle?: string; /** A provider's, not in the library yet. */ isExternal?: boolean; source?: string | null; sourceMarked?: boolean };
/** What a card plays whole. */
export type Whole = "album" | "playlist" | "artist" | "mix";
/** A mix on Home's "For you" (the core's `mix_cards`): `name` the core's `MixName`, up to four covers, and
 * its card's paper and ink (`mix_card_colours`). */
export type MixTile = { id: string; name: string; favourites: boolean; covers: string[]; page: string; ink: string };
/** A mix's page (the core's `mix_page`): the songs as they play. */
export type MixPage = { id: string; name: string; favourites: boolean; seconds: number; covers: string[]; songs: Song[] };
export type Artist = { id: string; name: string; cover?: string; albumCount?: number | null; starred?: boolean; isExternal?: boolean; source?: string | null; sourceMarked?: boolean };
export type Playlist = { id: string; name: string; songCount: number; duration: number; cover?: string; comment?: string; owner?: string; /** The account may change it (the core's `playlist_editable`). */ editable?: boolean; /** Pinned in this browser. */ pinned?: boolean; /** A provider's, not in the library yet. */ remote?: boolean; source?: string | null; sourceMarked?: boolean; mixed?: boolean };
/** A playlist songs can go into: `adds`, the ones it lacks; `holds`, how many it has already. */
export type PlaylistChoice = { playlist: Playlist; adds: string[]; holds: number };
/** As the server names it: the name is `value`. */
/** A genre with songs, as the core lists them (`genres_listed`): a cover of its songs where it has one. */
export type Genre = { name: string; songCount: number; cover?: string };
/** A decade with songs in the index: its first year. */
export type Decade = { start: number; songCount: number };

/** A song of the queue the player shows, in play order: its list `index`, who asked for it in a jam. */
export type QueueSong = Partial<Song> & { index: number; by?: string };

export type Now = {
  state: "Idle" | "Loading" | "Playing" | "Paused" | "Stopped" | string;
  /** The play button shows playing. */
  playing: boolean;
  index: number | null;
  id: string | null;
  positionMs: number;
  /** Paused while the fade is still heard: how much longer the place runs on, ms. */
  fadingMs: number;
  pace: number;
  mixing: boolean;
  buffering: boolean;
  song: Song | null;
  queue: QueueSong[];
  /** The place in `queue` of the song heard; -1 for none. */
  current: number;
  /** Where what plays next comes from: the album every song after the one heard shares, if they do. */
  from: string | null;
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
  /** The browser's media session shows it: not while another device plays. */
  system: boolean;
  /** A jam guest's picks are asked of the host; its lyric taps reach nothing. */
  asks: boolean;
  underruns: number;
  transition?: { kind: string; into: string; reason: string } | null;
};

// Field names as nori-model's serde has them.
export type LyricLine = { start_ms: number; end_ms: number; text: string; translation?: string | null; background?: boolean; backing?: string; voice?: number };
export type Lyrics = { synced: boolean; word_timed: boolean; lines: LyricLine[] };
export type Timing = { kind: "NotChecked" | "DoesNotFit" | "Unsure" | "Automatic" | "Yours"; ms: number; automatic?: number | null };
/** The lyrics shown for a song, where they came from and how they are timed. */
export type Shown = { song: string; lyrics: Lyrics; origin: string; timing: Timing };
/** What the session's lyrics view shows for the song heard (`LyricsNow`): looked up, failed, found. */
export type LyricsNow = { song: string | null; state: "looking" | "failed" | "none" } | ({ state: "shown" } & Shown);

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

export type Devices = { on: boolean; me?: string; here: boolean; active: string | null; list: { id: string; name: string; kind: string; playing: boolean; nearby: boolean; refused: string | null }[] };

export type SettingRow = {
  item:
    | { kind: "setting"; name: string; type: "Switch" | "Choice" | "Level" | "Text" | "Colour"; options: string[]; min: number; max: number; value: string; chips: boolean }
    | { kind: "note"; note: string }
    | { kind: "link"; to: string }
    /** What this browser keeps: the total, then each part's share of the bar and its size. */
    | { kind: "stored"; total: string; parts: { part: string; share: number; size: string }[] }
    | { kind: "unsent"; waiting: number; dropped: number }
    /** A chore, `size` what it clears. */
    | { kind: "action"; action: "ClearLyrics"; size: string };
  shown: boolean;
  enabled: boolean;
  depth: number;
};
export type SettingsSection = { id: string; rows: SettingRow[] };

/** A line of a song's menu (`menus::song_menu`); `place` where it is drawn (`SongMenuPlace`). */
export type MenuAction = { action: string; place: "Title" | "Names" | "Glyphs" | "Line"; more: boolean; enabled: boolean; on?: boolean; id?: string; name?: string; named?: boolean };

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

/** What a profile's circle shows (the core's `ProfileIcon`): its letter, its provider's mark, or a face. */
export type ProfileIcon = "Letter" | "Provider" | "Sing" | "Surprise" | "Talking" | "LookUp" | "Satisfied" | "Think";
/** A saved profile (the worker's `profiles`): `label` its name or its address's host, `letter` the one
 * `Letter` shows, `on` the one open. */
export type Profile = { id: string; kind: string; label: string; user: string; host: string; icon: ProfileIcon; letter: string; on: boolean; sources?: string[] };
/** The saved profiles in their order, the icons a circle can show in the picker's order, and whether they may
 * be changed here (a jam guest's are its host's). */
export type Profiles = { picks: boolean; icons: ProfileIcon[]; list: Profile[] };
/** A profile's metadata and merged sources (`profile_sources`): those it plays from, and saved profiles it can take. */
export type ProfileSources = { profile: Profile; shown: boolean; sources: Profile[]; offered: Profile[] };
/** One of the active server's music libraries (`music_folders`): whether it is shown, and the
 * `musicFolders` value switching it stores. */
export type MusicFolderSwitch = { id: string; name: string; on: boolean; tapped: string };
/** Where a profile's plays go (`scrobblers`): its own server (null where it takes none), then its accounts. */
export type Scrobbling = { itself: { on: boolean } | null; list: Scrobbler[] };
export type ScrobblerKind = "listenbrainz" | "librefm" | "lastfm";
/** What adding an account asks (`scrobbler_kinds`), in order. */
export type ScrobblerField = "token" | "apiKey" | "secret" | "user" | "password" | "address";
export type ScrobblerKindForm = {
  kind: ScrobblerKind;
  /** Signing in with what the form asks, or on the service's page. */
  signIn: "form" | "browser";
  /** Where the form's fields come from, if anywhere. */
  help: string | null;
  fields: { id: ScrobblerField; required: boolean; advanced: boolean }[];
};
/** A scrobbling account of a profile: `host` is empty for the kind's own service. */
export type Scrobbler = { id: string; kind: ScrobblerKind; user: string; host: string; on: boolean };
