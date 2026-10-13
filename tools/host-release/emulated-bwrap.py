#!/usr/bin/python3
"""Run Flatpak package checks under CPU emulation without a native syscall filter."""
import os
import sys

arguments = []
pending = iter(sys.argv[1:])
for argument in pending:
    if argument == "--args":
        with os.fdopen(int(next(pending)), "rb", closefd=False) as file:
            parts = file.read().split(b"\0")
            if parts[-1] == b"":
                parts.pop()
            arguments.extend(part.decode() for part in parts)
    else:
        arguments.append(argument)

filtered = []
pending = iter(arguments)
for argument in pending:
    if argument == "--":
        filtered.extend([argument, *pending])
        break
    if argument in ("--seccomp", "--add-seccomp-fd"):
        next(pending)
    else:
        filtered.append(argument)

os.execv("/usr/bin/bwrap", ["bwrap", *filtered])
