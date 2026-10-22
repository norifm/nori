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
};
export type Album = { id: string; name: string; artist: string; artistId?: string; year: number; cover?: string; songCount: number; duration: number; genre?: string; starred?: boolean };
export type Artist = { id: string; name: string; cover?: string; albumCount?: number | null; starred?: boolean };
export type Playlist = { id: string; name: string; songCount: number; duration: number; cover?: string; comment?: string; owner?: string };
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
  item: { kind: "setting"; name: string; type: "Switch" | "Choice" | "Level" | "Text" | "Colour"; options: string[]; min: number; max: number; value: string; chips: boolean } | { kind: "note"; note: string };
  shown: boolean;
  enabled: boolean;
  depth: number;
};
export type SettingsSection = { id: string; rows: SettingRow[] };

export type MenuAction = { action: string; more: boolean; enabled: boolean; on?: boolean; id?: string; name?: string; named?: boolean };
