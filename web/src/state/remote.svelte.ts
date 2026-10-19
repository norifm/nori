// Jams, as Spotify and the Android app have them, and moving the music between the account's devices
// ("Play On"). The rules are the core's (the remote in the player worker); this reads them and asks.

import * as rpc from "../rpc";
import { ask, facts } from "../rpc";
import { W } from "../words";
import { library } from "./library.svelte";
import { player } from "./player.svelte";
import { go } from "./route.svelte";
import type { Devices, Jam } from "./types";

class Remote {
  /** The jam; null before the player runs. */
  jam = $state.raw<Jam | null>(null);
  devices = $state.raw<Devices>({ on: false, active: null, list: [] });
  private keys = ["", ""];

  /** Reads the jam and the devices; each drawn again only when it changed. */
  async read() {
    if (!rpc.player) return;
    const [j, d] = await Promise.all([ask<string>("jam"), ask<string>("devices", JSON.stringify(W.kinds))]);
    if (j !== this.keys[0]) {
      this.keys[0] = j;
      const jam = JSON.parse(j) as Jam | null;
      this.jam = jam;
      facts.jam = jam && { start: jam.start, hosting: jam.hosting, guest: jam.guest, members: jam.members.length, pending: jam.pending.length, listening: jam.listening, link: jam.link };
    }
    if (d !== this.keys[1]) {
      this.keys[1] = d;
      this.devices = JSON.parse(d);
      facts.devices = this.devices;
    }
  }

  /** `f`, then the jam and devices read again. */
  async act(f: () => Promise<unknown>) {
    await f();
    await this.read();
  }

  /** Moves the music to device `id`, or here (""). */
  async pick(id: string) {
    // Music moving here plays here: the sound card is made in this click.
    if (!id) await player.soundCard();
    await this.act(() => ask("pick", id));
    await player.refresh();
  }

  /** Joins the jam `link` invites to: "joined" (the guest's profile is open), "own" or "ended". */
  async join(link: string, name: string): Promise<string> {
    const said = await ask<string>("jam_join", link.trim(), name.trim() || W.jamGuestName);
    if (said === "joined") this.switched();
    return said;
  }

  async leave() {
    if (await ask<boolean>("jam_leave")) this.switched();
    else location.reload();
  }

  async listen(on: boolean) {
    // Listening along plays here: the sound card is made in this click.
    if (on) await player.soundCard();
    await this.act(() => ask("jam_listen", on));
  }

  /** The library and the player are another profile's now: the pages read themselves again. */
  private switched() {
    go("home");
    library.changed();
    void player.refresh();
    void this.read();
  }

  follow() {
    rpc.onPlayerUp(() => void this.read());
    rpc.onSaid((s) => {
      if (s.kind === "remote") void this.read();
      // A jam this guest was in ended: its profile goes, the one before opens again.
      else if (s.kind === "jamEnded") void this.leave();
    });
  }
}

export const remote = new Remote();
