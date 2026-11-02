#!/usr/bin/env python3
"""A CPU profile of any command: samply samples it, and this writes, into the output folder,

  profile.json.gz   samply's profile (`samply load profile.json.gz` opens it in the Firefox profiler)
  stacks.folded     one line per stack, weighted by CPU microseconds (inferno and speedscope read it)
  flame.svg         the flamegraph (needs inferno-flamegraph: `cargo install inferno`)

and prints the functions taking the most CPU, by self and by total time.

  tools/flame.py [-o dir] [--rate hz] [--thread name] [--top n] -- command [args...]
  tools/flame.py --folded stacks.folded [--unit bytes] [--top n]   # the table from earlier stacks

Build what is profiled with symbols (`--profile profiling` in Cargo.toml). tools/flame-engine.sh runs it
over the engine's perf bench.
"""
import argparse
import collections
import gzip
import json
import os
import shutil
import subprocess
import sys


def symbols(syms_path):
    """{(library debug name, address): function name} from samply's presymbolicated sidecar."""
    if not os.path.exists(syms_path):
        return {}, {}
    data = json.load(open(syms_path))
    names = data["string_table"]
    by_lib = {}
    for lib in data["data"]:
        table = lib["symbol_table"]
        by_lib[lib["debug_name"]] = {addr: names[table[i]["symbol"]] for addr, i in lib["known_addresses"]}
    return by_lib, names


def folded(profile_path, thread_filter):
    """Counter of 'thread;outer;...;inner' -> CPU µs (or samples when the CPU delta is missing)."""
    profile = json.load(gzip.open(profile_path))
    by_lib, _ = symbols(profile_path.removesuffix(".gz").removesuffix(".json") + ".json.syms.json")
    libs = profile["libs"]
    stacks = collections.Counter()
    for t in profile["threads"]:
        name = t.get("name") or t.get("processName") or "thread"
        if thread_filter and thread_filter not in name:
            continue
        strings, frames, funcs, res = t["stringArray"], t["frameTable"], t["funcTable"], t["resourceTable"]

        def frame_name(f):
            func = frames["func"][f]
            r = funcs["resource"][func]
            if r is not None and r >= 0:
                lib = libs[res["lib"][r]]["debugName"]
                hit = by_lib.get(lib, {}).get(frames["address"][f])
                if hit:
                    return hit
                return f"{lib}+{strings[funcs['name'][func]]}"
            return strings[funcs["name"][func]]

        st = t["stackTable"]
        memo = {}

        def path(s):
            if s in memo:
                return memo[s]
            names = []
            k = s
            while k is not None:
                names.append(frame_name(st["frame"][k]).replace(";", ":"))
                k = st["prefix"][k]
            memo[s] = ";".join(reversed(names))
            return memo[s]

        samples = t["samples"]
        cpu = samples.get("threadCPUDelta")
        weight = samples.get("weight")
        for i, s in enumerate(samples["stack"]):
            if s is None:
                continue
            w = cpu[i] if cpu and cpu[i] is not None else (weight[i] if weight else 1)
            if w:
                stacks[f"{name};{path(s)}"] += w
    return stacks


def table(stacks, top, unit="us"):
    total = sum(stacks.values()) or 1
    scale, name = (1000, "ms") if unit == "us" else (1024, "KB")
    own, incl = collections.Counter(), collections.Counter()
    for line, w in stacks.items():
        frames = line.split(";")[1:]
        own[frames[-1]] += w
        for f in set(frames):
            incl[f] += w
    print(f"total {total / scale:.1f} {name}" + (" of CPU" if unit == "us" else " allocated"))
    for title, c in (("self", own), ("total", incl)):
        print(f"\n-- by {title}")
        for f, w in c.most_common(top):
            print(f"{100 * w / total:6.2f}%  {w / scale:9.1f} {name}  {f[:150]}")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("-o", "--out", default="build/flame")
    ap.add_argument("--rate", type=int, default=2000)
    ap.add_argument("--thread", help="only threads whose name contains this")
    ap.add_argument("--top", type=int, default=30)
    ap.add_argument("--folded", help="print the table from this stacks.folded and stop")
    ap.add_argument("--unit", choices=["us", "bytes"], default="us", help="what --folded counts")
    ap.add_argument("command", nargs=argparse.REMAINDER)
    a = ap.parse_args()
    if a.folded:
        stacks = collections.Counter()
        for line in open(a.folded):
            stack, _, w = line.rstrip().rpartition(" ")
            stacks[stack] += int(w)
        table(stacks, a.top, a.unit)
        return
    cmd = a.command[1:] if a.command[:1] == ["--"] else a.command
    if not cmd:
        ap.error("no command")
    if not shutil.which("samply"):
        sys.exit("samply is not installed: cargo install samply")
    os.makedirs(a.out, exist_ok=True)
    prof = os.path.join(a.out, "profile.json.gz")
    subprocess.run(["samply", "record", "-s", "-n", "--unstable-presymbolicate", "-r", str(a.rate), "-o", prof, *cmd], check=True)
    stacks = folded(prof, a.thread)
    path = os.path.join(a.out, "stacks.folded")
    with open(path, "w") as f:
        for line, w in sorted(stacks.items()):
            f.write(f"{line} {w}\n")
    if shutil.which("inferno-flamegraph"):
        with open(path) as i, open(os.path.join(a.out, "flame.svg"), "w") as o:
            subprocess.run(["inferno-flamegraph", "--title", " ".join(cmd)[:120], "--countname", "us"], stdin=i, stdout=o, check=True)
    table(stacks, a.top)
    print(f"\n{a.out}/flame.svg, {path}, {prof}")


if __name__ == "__main__":
    main()
