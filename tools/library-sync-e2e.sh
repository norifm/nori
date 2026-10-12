#!/usr/bin/env bash
# Incoming LAN favorites crossing JNI must update Compose state and media session buttons.
NORI_E2E_SERVER=${NORI_E2E_SERVER:-local}
source "$(dirname "$0")/e2e-lib.sh"
[[ "$NORI_E2E_SERVER" = local && "$ANDROID_SERIAL" = emulator-* ]] || { echo "requires the local server and an Android emulator" >&2; exit 2; }
local_server_up
root_before=$(adb shell id -u | tr -d '\r')
forward=""
cleanup_library_sync() {
  cleanup_e2e
  [ -z "$forward" ] || adb forward --remove "tcp:$forward" >/dev/null 2>&1
  if [ "$root_before" != 0 ]; then adb unroot >/dev/null; adb wait-for-device; fi
}
trap cleanup_library_sync EXIT
adb root >/dev/null
adb wait-for-device
app_up >/dev/null
remember_settings remoteControl
"$app" set remoteControl false >/dev/null
"$app" play "$PLAIN" >/dev/null
check 'a local song plays before the library update' sounds 15
song=$(field songId); original=$(field starred)
ports() { adb shell ss -ltn | python3 -c 'import sys; print(" ".join(line.split()[3].rsplit(":",1)[1] for line in sys.stdin if line.startswith("LISTEN")))'; }
before=$(ports)
"$app" set remoteControl true >/dev/null
door_started() {
  candidate=$(NORI_PORTS_BEFORE="$before" NORI_PORTS_NOW="$(ports)" python3 -c 'import os; print(" ".join(set(os.environ["NORI_PORTS_NOW"].split())-set(os.environ["NORI_PORTS_BEFORE"].split())))')
  [[ "$candidate" =~ ^[0-9]+$ ]]
}
check 'Android opens its authenticated local door' wait_until 10 door_started || exit 1
forward=$(adb forward tcp:0 "tcp:$candidate")
changed=$([ "$original" = True ] && echo False || echo True)
for state in "$changed" "$original"; do
  python3 - "$forward" "$song" "$state" <<'PY'
import hashlib, hmac, json, sys, time, urllib.request
port, song, state = sys.argv[1:]
body=json.dumps({'body':{'t':'command','id':time.time_ns(),'op':{'op':'libraryMarks','marks':[{'kind':'song','id':song,'on':state=='True'}]}}},separators=(',',':')).encode()
target='/rest/noriRemote.send?dev=favorite-fixture&ts='+str(time.time_ns()//1000000)
sig=hmac.new(b'nori-e2e',b'POST '+target.encode()+b'\n'+body,hashlib.sha256).hexdigest()
request=urllib.request.Request('http://127.0.0.1:'+port+target+'&sig='+sig,data=body,method='POST')
with urllib.request.urlopen(request,timeout=10) as response:
    assert response.status==200
PY
  check "incoming library mark updates the Android heart ($state)" wait_for starred "$state" 10
  notification_marked() {
    local buttons; buttons=$(field notification)
    [[ "$buttons" == *heart* ]] && { [[ "$state" = True && "$buttons" == *heart_filled* ]] || [[ "$state" = False && "$buttons" != *heart_filled* ]]; }
  }
  check "the media notification follows the received mark ($state)" wait_until 10 notification_marked
done
adb forward --remove "tcp:$forward"
forward=""
finish
