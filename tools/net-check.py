#!/usr/bin/env python3
"""What the app takes from the network while it plays, on a device (ANDROID_SERIAL): the bytes the device
received, against each song the player opened (from which byte, how many bytes it was promised). A song
opened from its first byte twice, or a device that received more than the songs it opened, is music
fetched twice; one opened again from the cache costs no network.

  tools/net-check.py [seconds=180] [play]   play: anything tools/app.sh play takes, played first

Only this app should use the network meanwhile (an emulator's own services are quiet enough).
"""
import collections
import os
import re
import subprocess
import sys
import time

OPENED = re.compile(r"rust player: (\S+?):(\S*) from byte (\d+): (\d+) bytes come")


def adb(*args):
    return subprocess.run(["adb", "shell", *args], capture_output=True, text=True).stdout


def received():
    """Bytes received on every interface but loopback."""
    total = 0
    for line in adb("cat", "/proc/net/dev").splitlines()[2:]:
        name, _, rest = line.partition(":")
        if name.strip() != "lo" and rest.split():
            total += int(rest.split()[0])
    return total


def main():
    seconds = int(sys.argv[1]) if len(sys.argv) > 1 else 180
    adb("logcat", "-c")
    before = received()
    if len(sys.argv) > 2:
        here = os.path.dirname(os.path.abspath(__file__))
        subprocess.run([os.path.join(here, "app.sh"), "play", sys.argv[2]], check=False, capture_output=True)
    time.sleep(seconds)
    got = received() - before
    log = subprocess.run(["adb", "logcat", "-d", "-s", "nori:I"], capture_output=True, text=True).stdout
    opens = collections.defaultdict(list)
    for m in OPENED.finditer(log):
        opens[m[1]].append((int(m[3]), int(m[4])))
    promised = sum(n for runs in opens.values() for _, n in runs)
    whole = sum(max(n for at, n in runs if at == 0) if any(at == 0 for at, _ in runs) else 0 for runs in opens.values())
    print(f"{seconds} s: the device received {got / 1e6:.1f} MB; the player opened {len(opens)} songs, "
          f"{promised / 1e6:.1f} MB asked for, {whole / 1e6:.1f} MB of them from the first byte")
    for song, runs in opens.items():
        again = sum(1 for at, _ in runs if at == 0) > 1
        print(f"  {song}: " + ", ".join(f"from {at} ({n / 1e6:.2f} MB)" for at, n in runs) + ("  <- from the start again" if again else ""))


if __name__ == "__main__":
    main()
