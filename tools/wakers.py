#!/usr/bin/env python3
"""Who wakes the app's threads, from a scheduler trace of the device (atrace sched): per thread of nori,
wakes per minute and the threads that woke it. A thread woken from another process's binder thread is
mostly that thread answering a call it made (a reply wakes the caller). A wake costs a phone battery even
with little CPU: with the screen off, each one can take a core out of its deepest idle.

  tools/wakers.py [seconds=60]          record on the device (ANDROID_SERIAL), then print
  tools/wakers.py --trace FILE PID      print from an earlier trace

Turn the screen off first (adb shell input keyevent 223) to see what runs with it off.
"""
import collections
import os
import re
import subprocess
import sys
import tempfile

LINE = re.compile(r"^\s*(.+?)-(\d+)\s+\(\s*(\d+|-+)\)\s+\[\d+\]\s+\S+\s+([\d.]+): (\w+): (.*)$")
WOKEN = re.compile(r"comm=(.+?) pid=(\d+) ")


def report(path, pid):
    names, wakes, first, last = {}, [], None, None
    for line in open(path, errors="replace"):
        m = LINE.match(line)
        if not m:
            continue
        comm, tid, tgid, ts, event, rest = m.groups()
        ts = float(ts)
        first = ts if first is None else first
        last = ts
        if tgid == pid:
            names[tid] = comm
        if event in ("sched_waking", "sched_wakeup"):
            w = WOKEN.search(rest)
            if w:
                wakes.append((comm, tgid, w.group(2), event))
    if first is None:
        sys.exit("no scheduler events in the trace")
    # sched_waking is the waker's own event; older kernels have only sched_wakeup.
    kind = "sched_waking" if any(w[3] == "sched_waking" for w in wakes) else "sched_wakeup"
    minutes = max(last - first, 1e-9) / 60
    by_thread = collections.defaultdict(collections.Counter)
    for comm, tgid, woken, event in wakes:
        if event == kind and woken in names:
            by_thread[woken][comm + (" (app)" if tgid == pid else "")] += 1
    total = sum(sum(c.values()) for c in by_thread.values())
    print(f"{(last - first):.0f} s traced: the app's threads woke {total / minutes:.0f} times a minute")
    for tid, c in sorted(by_thread.items(), key=lambda kv: -sum(kv[1].values())):
        woke = ", ".join(f"{w} {n / minutes:.0f}" for w, n in c.most_common(5))
        print(f"  {names[tid] + ':' + tid:28s} {sum(c.values()) / minutes:6.0f}/min  by {woke}")


def main():
    if sys.argv[1:2] == ["--trace"]:
        report(sys.argv[2], sys.argv[3])
        return
    seconds = sys.argv[1] if len(sys.argv) > 1 else "60"
    pkg = os.environ.get("NORI_PKG", "dev.nori.music")
    pid = subprocess.run(["adb", "shell", "pidof", pkg], capture_output=True, text=True).stdout.strip()
    if not pid:
        sys.exit(f"{pkg} is not running")
    with tempfile.NamedTemporaryFile("w+", suffix=".trace") as f:
        subprocess.run(["adb", "shell", f"atrace -t {seconds} -b 16384 sched"], stdout=f, check=True)
        f.flush()
        report(f.name, pid)


if __name__ == "__main__":
    main()
