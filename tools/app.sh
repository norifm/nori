#!/usr/bin/env bash
# Drives a debug build of nori directly, instead of tapping screen coordinates:
#   tools/app.sh open settings/sound      navigate to a route
#   tools/app.sh play "search:noise 1"    play a song by id, album, or first search hit
#   tools/app.sh set limiter true         flip one setting
#   tools/app.sh state                    one JSON line: route, playback, key settings
# Needs the app running (tools/app.sh launch). Answers come back from logcat, tag noritest.
set -uo pipefail
# The one emulator, unless told otherwise: a second device attached must never be driven by accident.
export ANDROID_SERIAL=${ANDROID_SERIAL:-emulator-5554}
pkg=${NORI_PKG:-dev.nori.music}
send() {
  local request cmdline
  request=$(python3 -c 'import uuid; print(uuid.uuid4().hex)')
  cmdline=$(python3 - "$pkg" "$request" "$@" <<'PYTHON'
import shlex
import sys
pkg, request, *args = sys.argv[1:]
command = ['am', 'broadcast', '-n', f'{pkg}/dev.nori.music.app.TestBridge',
           '-a', 'dev.nori.music.TEST', '--es', 'request', request]
for key, value in zip(('cmd', 'arg', 'value'), args):
    command.extend(['--es', key, value])
print(shlex.join(command))
PYTHON
  )
  adb shell "$cmdline" >/dev/null 2>&1 || return 1
  for _ in $(seq 20); do
    local reply
    reply=$(adb logcat -d -s noritest:I | sed -n "s/^.*noritest: $request //p" | head -1)
    if [ -n "$reply" ]; then
      printf '%s\n' "$reply"; return 0
    fi
    sleep 0.25
  done
  echo "no answer (is a debug build running?)" >&2; return 1
}
case "${1:-}" in
  # Up as soon as the screen answers (a route other than the service's "background"), 20 s at most.
  # The launcher's own activity, started as a tap on its icon would: monkey refuses to run on images that
  # declare no hardware keys ("SYS_KEYS has no physical keys"), which newer emulators do.
  launch) main=$(adb shell cmd package resolve-activity --brief -c android.intent.category.LAUNCHER $pkg | tr -d '\r' | tail -1)
    adb shell am start -n "$main" >/dev/null 2>&1
    for _ in $(seq 40); do
      s=$(send state 2>/dev/null) && [[ "$s" == *'"route"'* && "$s" != *'"route":"background"'* ]] && { echo "$s"; exit 0; }
      sleep 0.5
    done
    send state ;;
  open|play|state|set|login|do|remote|queuesaved|webcookies) send "$@" ;;
  wake) adb shell input keyevent 224 >/dev/null; adb shell wm dismiss-keyguard >/dev/null 2>&1; sleep 1 ;;
  *) echo "usage: app.sh launch|open <route>|play <ref>|do <action>|login <url|user|pass>|login kind:<KIND>|set <name> <value>|state|queuesaved|webcookies|wake" >&2; exit 2 ;;
esac
