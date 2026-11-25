#!/usr/bin/env python3
"""Drive a test-control desktop build through its private Unix socket."""
import argparse
import json
import os
import socket

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--socket", default=os.environ.get("NORI_DESKTOP_SOCKET"))
parser.add_argument("command", choices=["state", "open", "do", "panel", "play_album", "tab", "shot", "profiles", "login", "setup_answer", "setup_back", "appearance", "click", "welcome_pick", "welcome_continue", "sync",
                                        "devices_page", "pair_open", "pair_role", "pair_paste", "pair_confirm", "pair_close", "setting_toggle", "setting_action", "ask_text", "ask_answer"])
parser.add_argument("arg", nargs="*")
args = parser.parse_args()
if not args.socket:
    parser.error("set NORI_DESKTOP_SOCKET or pass --socket")
bare = ("state", "profiles", "setup_back", "welcome_continue", "devices_page", "pair_open", "pair_close")
# Those answered yes or no.
flags = ("pair_confirm", "ask_answer")
if args.command == "sync":
    # sync state|code|take <link>|confirm [keep]|cancel|reset
    if not args.arg or args.arg[0] not in ("state", "code", "take", "confirm", "cancel", "reset"):
        parser.error("sync takes state, code, take <link>, confirm [keep], cancel or reset")
    do, rest = args.arg[0], args.arg[1:]
    ask = {"do": do}
    if do == "take":
        ask["arg"] = rest[0]
    elif do == "confirm":
        ask["arg"] = rest == ["keep"]
    request = {"sync": ask}
elif args.command == "setting_toggle":
    if len(args.arg) != 2:
        parser.error("setting_toggle takes a name and on or off")
    request = {"command": "setting_toggle", "arg": [args.arg[0], args.arg[1] == "on"]}
elif args.command in flags:
    if len(args.arg) != 1 or args.arg[0] not in ("yes", "no"):
        parser.error(f"{args.command} takes yes or no")
    request = {"command": args.command, "arg": args.arg[0] == "yes"}
elif len(args.arg) != (0 if args.command in bare else 3 if args.command == "login" else 2 if args.command == "click" else 1):
    parser.error("state, profiles, setup_back and welcome_continue take no argument; login takes a URL, a user and a password; click x and y; the others one")
else:
    request = {"command": args.command}
    if args.command == "login":
        request["arg"] = dict(zip(("url", "user", "password"), args.arg))
    elif args.command == "click":
        request["arg"] = [float(a) for a in args.arg]
    elif args.arg:
        arg = args.arg[0]
        request["arg"] = int(arg) if args.command in ("tab", "setup_answer", "appearance", "welcome_pick", "pair_role") else os.path.abspath(arg) if args.command == "shot" else arg
try:
    with socket.socket(socket.AF_UNIX) as client:
        client.settimeout(5)
        client.connect(args.socket)
        client.sendall((json.dumps(request) + "\n").encode())
        with client.makefile() as reply:
            state = json.loads(reply.readline())
    print(json.dumps(state))
    if "error" in state:
        raise SystemExit(1)
except (OSError, ValueError) as error:
    parser.exit(1, f"desktop control: {error}\n")
