// The player's sleeve a plugin gave for the song playing. Apart from the plugin host, which only loads when
// a plugin is installed.

export type Sleeve = { song: string; video?: string; still?: string; poster?: string };

class Sleeves {
  current = $state.raw<Sleeve | null>(null);
  /** The video couldn't play: `status` is its answer, if it had one. */
  failed: (song: string, status: number) => void = () => {};
}

export const sleeves = new Sleeves();
