// Pairing and profile sync, as Android's SyncViewModel: where sync stands (the worker's `sync_state`), read
// again whenever sync says something changed; what happened in the group as a notice. The rules are the
// core's; this reads them and asks.

import * as rpc from "../rpc";
import { ask, facts } from "../rpc";
import { W } from "../words";
import { ui } from "./ui.svelte";

/** Where sync stands, as the worker's `sync_state` says. */
export type SyncView = {
  paired: boolean;
  on: boolean;
  server: string;
  /** The side a pairing screen opens on (the core's `pair_side`). */
  opens: "Show" | "Take";
  /** This browser has profiles to give. */
  gives: boolean;
  state: string;
  refused?: string;
  retryMs?: number;
  pairing: string;
  code?: string;
  words?: string[];
  confirm?: { name: string; profiles: number; leaves: number | null; words: string[] | null; asksKeepCopy: boolean };
  reason?: string;
  serverRefused?: string;
  devices: { name: string; online: boolean; lastMs: number }[];
  keepsCopy: boolean;
  offers: { a: string; b: string }[];
  synced: string[];
};

class Sync {
  view = $state.raw<SyncView | null>(null);
  /** The Pair a device sheet is open. */
  pairing = $state(false);
  /** Bumped when sync changed the saved profiles: the pages listing them read them again. */
  profiles = $state(0);

  async read() {
    const v = JSON.parse(await ask<string>("sync_state")) as SyncView;
    this.view = v;
    facts.sync = v;
    return v;
  }

  /** Worker call `call`, then where sync stands read again. */
  async act(call: string, ...args: unknown[]) {
    await ask(call, ...args);
    return this.read();
  }

  /** Use a sync server on or off: sync starts or stops with it. */
  on(on: boolean) {
    return this.act("sync_on", on);
  }

  /** The sync server's address; blank for nori's own. */
  server(address: string) {
    return this.act("sync_server", address);
  }

  /** Listens to sync: `profilesChanged` runs when sync changed the profile open, or the list. */
  follow(profilesChanged: (open: boolean) => void) {
    rpc.onSaid((s) => {
      if (s.kind !== "sync") return;
      const event = s.event as { kind: string; name?: string; refused?: string } | undefined;
      const said = event && W.syncEvent(event);
      if (said) ui.say(said);
      if (typeof s.profiles === "string") {
        this.profiles++;
        profilesChanged(s.profiles !== "Same");
      }
      void this.read().catch(rpc.unlessElsewhere);
    });
  }
}

export const sync = new Sync();
