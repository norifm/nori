#!/usr/bin/env bash
# Incoming LAN favorites crossing JNI must update Compose state and media session buttons.
NORI_E2E_SERVER=${NORI_E2E_SERVER:-local}
source "$(dirname "$0")/e2e-lib.sh"
[[ "$NORI_E2E_SERVER" = local && "$ANDROID_SERIAL" = emulator-* ]] || { echo "requires the local server and an Android emulator" >&2; exit 2; }
local_server_up
root_before=$(adb shell id -u | tr -d '\r')
forward=""; sync_pid=""
cleanup_library_sync() {
  cleanup_e2e
  [ -z "$forward" ] || adb forward --remove "tcp:$forward" >/dev/null 2>&1
  if [ -n "$sync_pid" ]; then
    kill "$sync_pid" 2>/dev/null; adb reverse --remove "tcp:$sync_port" >/dev/null 2>&1; adb reverse --remove "tcp:$url_port" >/dev/null 2>&1
  fi
  if [ "$root_before" != 0 ]; then adb unroot >/dev/null; adb wait-for-device; fi
}
trap cleanup_library_sync EXIT
adb root >/dev/null
adb wait-for-device
app_up >/dev/null
# The door admits only the devices paired with this phone: a peer's data folder, paired through a sync
# server on this computer, signs as they do (tools/pair-peer.py).
peer="$here/../build/e2e/$ANDROID_SERIAL/library-peer"; rm -rf "$peer"; mkdir -p "$peer"
python3 "$here/with-resource.py" build cargo build -j4 -p nori-cli >/dev/null || exit 1
paired=$(python3 "$here/pair-peer.py" "$here/../target/debug/nori-cli" "$peer" http://localhost:4533 "$USER" "$PASS") || { check 'the phone pairs with a peer' false; exit 1; }
read -r sync_pid sync_port url_port <<<"$paired"
"$app" login "http://localhost:4533|$USER|$PASS" >/dev/null
check "the phone opens the group's profile" wait_for server http://localhost:4533 30 || exit 1
# What the group's devices sign with at this profile's doors: HKDF-SHA256 of the group key (nori-sync's
# GroupKey::lan_secret), named by the profile's sync identity.
secret=$(python3 - "$peer/nori.db" <<'PY'
import base64, hashlib, hmac, json, sqlite3, sys
with sqlite3.connect(sys.argv[1]) as db:
    kept = json.loads(db.execute("SELECT value FROM app_kv WHERE key='syncGroup'").fetchone()[0])
key = base64.urlsafe_b64decode(kept["group"]["key"] + "=")
sync_id = next(r["shared"]["id"] for r in kept["book"]["records"] if not r["off"])
prk = hmac.new(b"\0" * 32, key, hashlib.sha256).digest()
print(hmac.new(prk, f"nori lan door {sync_id}".encode() + b"\x01", hashlib.sha256).hexdigest())
PY
) || { check "the group's key is read" false; exit 1; }
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
  python3 - "$forward" "$song" "$state" "$secret" <<'PY'
import hashlib, hmac, json, sys, time, urllib.request
port, song, state, secret = sys.argv[1:]
body=json.dumps({'body':{'t':'command','id':time.time_ns(),'op':{'op':'libraryMarks','marks':[{'kind':'song','id':song,'on':state=='True'}]}}},separators=(',',':')).encode()
target='/rest/noriRemote.send?dev=favorite-fixture&ts='+str(time.time_ns()//1000000)
sig=hmac.new(secret.encode(),b'POST '+target.encode()+b'\n'+body,hashlib.sha256).hexdigest()
request=urllib.request.Request('http://127.0.0.1:'+port+target+'&sig='+sig,data=body,method='POST')
with urllib.request.urlopen(request,timeout=10) as response:
    assert response.status==200
PY
  check "incoming library mark updates the Android seal ($state)" wait_for starred "$state" 10
  notification_marked() {
    local buttons; buttons=$(field notification)
    [[ "$buttons" == *seal* ]] && { [[ "$state" = True && "$buttons" == *sealed* ]] || [[ "$state" = False && "$buttons" != *sealed* ]]; }
  }
  check "the media notification follows the received mark ($state)" wait_until 10 notification_marked
done
adb forward --remove "tcp:$forward"
forward=""
"$app" login "$APP_URL|$USER|$PASS" >/dev/null
check "the phone's own profile opens again" wait_for server "$APP_URL" 30
finish
