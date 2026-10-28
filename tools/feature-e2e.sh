#!/usr/bin/env bash
# The rest of the app where only a device can say: the screens' buttons and what they start, the
# notification's commands, downloads through media3 and its notifications, playing offline and the
# offline bridge as the network really goes, the mock DAC's track, and a device's sound on connect. The
# decisions behind them (stars, playlists, scrobbling, lyrics choice, mixes, what plays when the queue
# runs out, the DAC's modes, AutoEQ) are the core's and tested by cargo test against fake servers;
# docs/developers/testing.md lists what moved where. Checks that the server has to agree with ask the server's API.
#   tools/feature-e2e.sh [--only <section>,...] [--list]     NORI_E2E_SERVER=local for tools/dev-server.sh
#   (the real server's credentials come from ~/.music.pass: url, blank, user, password)
source "$(dirname "$0")/e2e-lib.sh"
SECTIONS="plugins lyrics notification album-page bridge download-notification downloads mobile-data car-art foryou dac device-sound sheets queue-panel remote jam"
OPT_IN="lyrics-services scrobble download-albums youtube-music"
list_sections
if { want scrobble || want download-albums; } && [ "$NORI_E2E_SERVER" != local ]; then
  echo "the scrobble and download-albums checks require NORI_E2E_SERVER=local" >&2
  exit 2
fi
json() { python3 -c "import sys,json;d=json.load(sys.stdin)['subsonic-response'];print(eval('d$1',{'d':d}))" 2>/dev/null; }
# What the screen itself reports, read out of the accessibility tree rather than guessed at from a
# screenshot: a label to assert on, and a node to press where the UI says the button is.
ui() { "$here/ui-dump.sh"; }
pill() { ui | grep -oE 'text="(Play|Pause)"' | head -1 | cut -d'"' -f2; }
pill_is() { [ "$(pill)" = "$1" ]; }
tapnode_now() { # $1 = text|content-desc, $2 = that value, $3 = which of several (the first by default)
  "$here/ui-dump.sh" --click "$1" "$2" "${3:-1}" >/dev/null 2>&1
}
tapnode() { wait_until 10 tapnode_now "$@"; }
on_screen() { ui | grep -q -- "$1"; }
off_screen() { local tree; tree=$(ui) || return 1; ! printf '%s' "$tree" | grep -q -- "$1"; }
car_read() {
  local parent="$1" before
  before=$(adb logcat -d -v brief -s noricar:I | wc -l | tr -d ' ')
  "$app" do "car tree $parent" >/dev/null
  car_done() {
    adb logcat -d -v brief -s noricar:I | tail -n "+$((before + 1))" > "$car_log"
    grep -Fq "done tree $parent" "$car_log"
  }
  wait_until 20 car_done && grep -Fq "tree $parent: code=0" "$car_log"
}
widget_albums() {
  local before
  before=$(adb logcat -d -v brief -s noriwidget:I | wc -l)
  "$app" do 'widgetalbums 6' >/dev/null
  widget_read() { adb logcat -d -v brief -s noriwidget:I | tail -n "+$((before + 1))" | grep -q 'albums '; }
  wait_until 30 widget_read || return 1
  adb logcat -d -v brief -s noriwidget:I | tail -1 | sed 's/.*albums //'
}
# Chrome may keep the landing page until its explicit app link is tapped.
open_invite() {
  local link="$1"
  [ -n "$link" ] || return 1
  if [ "${2:-}" = app ]; then
    link="nori://jam?${link#*#}"
    adb shell am start -W -a android.intent.action.VIEW -d "'$link'" >/dev/null 2>&1 || return 1
  else
    adb shell am start -W -a android.intent.action.VIEW -p com.android.chrome --ez create_new_tab true -d "'$link'" >/dev/null 2>&1 || return 1
  fi
  invite_opened() {
    local tree; tree=$(ui)
    if [[ "$tree" == *"package=\"$pkg\""* ]]; then return 0; fi
    for button in "Use without an account" "Stay signed out" "No thanks" "Got it" "Open in nori"; do
      if [[ "$tree" == *"text=\"$button\""* ]]; then tapnode text "$button"; break; fi
    done
    return 1
  }
  wait_until 15 invite_opened
}
starred_on_server() { api getStarred2 | python3 -c "
import sys,json
d=json.load(sys.stdin)['subsonic-response'].get('starred2',{})
print(any(s['id']=='$1' for s in d.get('song',[])))"; }

cleanup_feature() {
  [ "${feature_finished:-0}" = 0 ] || return 0
  if [ -n "${peer:-}" ]; then
    "$app" remote pick here >/dev/null
    "$app" set remoteControl false >/dev/null
    tmux kill-session -t "$peer" 2>/dev/null
  fi
  for guest_pid in "${gus:-}" "${dee:-}" "${host:-}"; do
    [ -z "$guest_pid" ] || kill "$guest_pid" 2>/dev/null
  done
  if [ -n "${app_jam:-}" ]; then
    "$app" login "$APP_URL|$USER|$PASS" >/dev/null
  fi
}

whole_run_log
echo "== features end to end against $([ "$NORI_E2E_SERVER" = local ] && echo "the local server" || echo "the real server")"
local_server_up
adb shell am force-stop "$pkg" >/dev/null 2>&1
app_up >/dev/null || { echo "the app did not come up"; exit 1; }
remember_settings offload autoMix eq remoteControl
id=$(long_song)

if want scrobble; then section scrobble
  remember_settings scrobble
  "$app" set scrobble true >/dev/null
  "$app" set autoMix false >/dev/null
  scrobble_first=$(song_id 'Long Track 01'); scrobble_second=$(song_id 'Long Track 02'); scrobble_third=$(song_id 'Long Track 03')
  count() { api getSong "&id=$1" | python3 -c 'import json,sys;print(json.load(sys.stdin)["subsonic-response"]["song"].get("playCount",0))'; }
  scrobble_before_first=$(count "$scrobble_first"); scrobble_before_second=$(count "$scrobble_second")
  "$app" play "song:$scrobble_first" >/dev/null
  check 'first track produces audio' sounds 15
  check 'first track starts' wait_for positionMs '>1000' 10
  "$app" play "song:$scrobble_second" >/dev/null
  check 'second track produces audio' sounds 15
  check 'an early skip does not count a play' test "$(count "$scrobble_first")" = "$scrobble_before_first"
  check 'second track advances' wait_for positionMs '>3000' 10
  "$app" do pause >/dev/null
  check 'pause stops audio' silent 10
  check 'pause does not submit a play' test "$(count "$scrobble_second")" = "$scrobble_before_second"
  "$app" do resume >/dev/null
  check 'resuming starts audio' sounds 10
  check 'half the song is heard' wait_for positionMs '>45000' 60
  "$app" play "song:$scrobble_third" >/dev/null
  count_is() { [ "$(count "$1")" = "$2" ]; }
  check 'the server counts the completed listen exactly once' wait_until 10 count_is "$scrobble_second" "$((scrobble_before_second + 1))"
  check 'the next track produces audio' sounds 10
  check 'the next track advances' wait_for positionMs '>2000' 10
  check 'the completed listen stays at one submission' test "$(count "$scrobble_second")" = "$((scrobble_before_second + 1))"
  "$app" do pause >/dev/null
fi

# The packed plugins (nori-plugins' tools/pack.py, dist/<name>-<version>.noriplugin).
plugins_dist=${NORI_PLUGINS_DIST:-$here/../../nori-plugins/dist}
plugin_package() { ls "$plugins_dist/$1"-[0-9]*.noriplugin 2>/dev/null | sort -V | tail -1; }
# Orbit's package as a repo of this computer's, over HTTP the emulator reaches: its index.json beside it.
serve_plugins() {
  local repo; repo=$(mktemp -d)
  cp "$(plugin_package orbit)" "$repo/" || return 1
  python3 - "$repo" <<'INDEX' || return 1
import hashlib, json, pathlib, sys
repo = pathlib.Path(sys.argv[1]); pkg = next(repo.glob("orbit-*.noriplugin"))
version = pkg.stem.removeprefix("orbit-")
(repo / "index.json").write_text(json.dumps({"plugins": [{"id": "fm.nori.orbit", "name": "Orbit", "versions": [
    {"version": version, "api": 1, "url": pkg.name, "sha256": hashlib.sha256(pkg.read_bytes()).hexdigest()}]}]}))
INDEX
  plugins_port=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
  python3 -m http.server "$plugins_port" --bind 127.0.0.1 --directory "$repo" >/dev/null 2>&1 &
  plugins_server=$!
  wait_until 10 curl -sf "http://127.0.0.1:$plugins_port/index.json" -o /dev/null
}
plugin_is() { "$app" plugin cards | grep -q "$1"; }
# Installs and turns on plugin $1's newest package, pushed into the app's cache: the id.
plugin_from_file() {
  adb exec-in run-as "$pkg" sh -c "cat > cache/$1.noriplugin" < "$(plugin_package "$1")" || return 1
  local id; id=$("$app" plugin install "/data/data/$pkg/cache/$1.noriplugin")
  [[ "$id" == *.* && "$id" != *failed* ]] || return 1
  "$app" plugin turn "$id on" >/dev/null && echo "$id"
}

if want plugins; then section "plugins: a repo added, orbit installed from it, turned on and off, its rows, removed; music sounding"
  # The plugin host's runtime on the device's ABI, the Plugins page and its sheets, a repo fetched through
  # OkHttp, and a plugin's sound stage reaching the AudioTrack player at once; what plugins may do is Rust's.
  check "orbit is packed" test -n "$(plugin_package orbit)"
  check "its repo is served" serve_plugins
  "$app" plugin reset >/dev/null
  "$app" play "$SONG" >/dev/null
  check "music sounds before any plugin" sounds 20
  "$app" open plugins >/dev/null
  check "the Plugins page opens, empty" wait_until 10 on_screen 'Plugins add music sources'
  shots="$here/../build/e2e/$ANDROID_SERIAL/plugins"; mkdir -p "$shots"
  adb exec-out screencap -p > "$shots/plugins-empty.png"
  tapnode text "Add repo"
  check "adding a repo warns first" wait_until 10 on_screen 'Add it only if you trust it'
  tapnode text "Repo address"
  adb shell input text "http://10.0.2.2:$plugins_port/index.json"
  tapnode text Add
  check "the repo offers orbit" wait_until 30 on_screen 'text="Orbit"'
  tapnode text Orbit
  check "installing shows what the repo says first" wait_until 10 on_screen 'shown before it’s turned on'
  adb exec-out screencap -p > "$shots/install-sheet.png"
  tapnode text Install
  check "orbit installs from the repo, off" wait_until 30 plugin_is 'fm.nori.orbit:off'
  check "and asks to turn it on, saying what it may do" wait_until 10 on_screen 'Change the sound'
  adb exec-out screencap -p > "$shots/turn-on-sheet.png"
  watch_from_now
  tapnode text "Turn on"
  check "orbit turns on" wait_until 10 plugin_is 'fm.nori.orbit:on'
  check "the player takes its stage in at once" waitfor_log 'the chain changes' 10
  check "installed, it leaves Get plugins" wait_until 10 off_screen 'text="Install"'
  check "music sounds with orbit's stage in the chain" sounds 10
  adb exec-out screencap -p > "$shots/plugins.png"
  watch_from_now
  "$app" plugin turn "fm.nori.orbit off" >/dev/null
  check "turned off, the player lets its stage go at once" waitfor_log 'the chain changes' 10
  watch_from_now
  "$app" plugin turn "fm.nori.orbit on" >/dev/null
  check "and takes it in again" waitfor_log 'the chain changes' 10
  tapnode text Orbit
  check "its page shows each level's value" wait_until 10 on_screen 'Turns per second · 0.12'
  adb exec-out screencap -p > "$shots/orbit.png"
  "$app" play "$SONG" >/dev/null
  check "a song started with orbit on sounds" sounds 10
  check "and plays on" wait_for positionMs '>3000' 15
  tapnode text Remove
  check "Remove asks first" wait_until 10 on_screen 'Its own settings go with it'
  watch_from_now
  tapnode text Remove
  check "removed, it is gone" wait_until 10 plugin_is none
  check "and so is its stage" waitfor_log 'the chain changes' 10
  check "the page goes back to the list" wait_until 10 on_screen 'Plugins add music sources'
  "$app" do pause >/dev/null
  "$app" plugin reset >/dev/null
  kill "$plugins_server" 2>/dev/null
  "$app" open settings >/dev/null
fi

if want lyrics; then section lyrics
  # Which answer wins, word timing, the lookups switch and every service's format are the lyrics crate's
  # (crates/lyrics tests, over recorded answers). Here: that an answer comes back through the platform.
  if [ "$NORI_E2E_SERVER" = local ]; then
    remember_settings lyricsOnline autoFill
    lyrics_fill=$(field autoFill | tr 'TF' 'tf')
    "$app" set autoFill false >/dev/null
    "$app" set lyricsOnline false >/dev/null
    "$app" play "$LYRICS_SONG" >/dev/null; sounds 20
    check "the lyrics fixture is the current song" wait_for title "Long Track 05" 10
    lyric_id=$(song_id "Long Track 05")
    "$app" do "lyrics song:$lyric_id" >/dev/null
    check "fixture lyrics arrive through the Android binding" wait_for lyricLines ">0" 10
    check "the fixture keeps its line timing" test "$(field lyricsSynced)" = True
    next_id=$(song_id "Long Track 06")
    downloaded_song() {
      [ "$(adb shell "run-as $pkg sqlite3 files/nori.db \"select count(*) from downloads where id='$next_id' and done=1\"" | tr -d '\r')" = "$1" ]
    }
    "$app" do "undownload song:$next_id" >/dev/null
    check "the upcoming fixture has no download" wait_until 10 downloaded_song 0
    # The songs ahead are warmed only while the lyrics are open (Client::lyrics_view).
    "$app" open lyrics >/dev/null
    watch_from_now
    "$app" do "enqueue song:$next_id" >/dev/null
    check "with the lyrics open, the upcoming lyrics finish preparing" waitfor_log "lyrics: prefetched $next_id" 20
    "$app" open home >/dev/null
    offline
    "$app" do "lyrics song:$next_id" >/dev/null
    check "the upcoming words are cached before downloading" wait_for lyricsAnswerSong "$next_id" 10
    check "the cached upcoming answer has lines" test "$(field lyricsAnswerLines)" -gt 0
    online
    "$app" do "download song:$next_id" >/dev/null
    check "the upcoming fixture song is downloaded" wait_until 30 downloaded_song 1
    offline
    "$app" do next >/dev/null
    check "the downloaded upcoming song plays offline" wait_for title "Long Track 06" 15
    "$app" do "lyrics song:$next_id" >/dev/null
    check "its prefetched lyrics cross the binding offline" wait_for lyricsSong "$next_id" 10
    check "cached lyrics retain their timing offline" test "$(field lyricsSynced)" = True
    online
    "$app" set autoFill "$lyrics_fill" >/dev/null
  else
    "$app" set thirdPartyLookups true >/dev/null
    "$app" play "$LYRICS_SONG" >/dev/null; sounds 20
    "$app" do lyrics >/dev/null
    check "lyrics arrive for a well-known song" wait_for lyricLines ">0" 30
    echo "     $(field lyricLines) lines from $(field lyricsSource)"
  fi
fi

if want lyrics-services; then section "each lyrics service (a report, not a check)"
  # Each service asked on its own for the same song: whether the services still answer is theirs to
  # say, not the app's. A service's answer and a failure are kept in the response cache, so a second run
  # reads what the first found: clear the app's data to ask them all again.
  "$app" set lyricsOnline true >/dev/null
  "$app" play "$LYRICS_SONG" >/dev/null; sounds 20
  answered=0
  services="lyrics_plus better_lyrics lrclib kugou netease binilyrics paxsenix unison paxsenix_musixmatch paxsenix_spotify youtube_music"
  for s in $services; do
    "$app" set lyricsSources "$s" >/dev/null
    "$app" do lyrics >/dev/null; wait_for lyricsSource "!SERVER" 12 2>/dev/null
    src=$(field lyricsSource)
    echo "     $s: $(field lyricLines) lines from $src (synced=$(field lyricsSynced), wordTimed=$(field lyricsWordTimed))"
    [ "$src" != "SERVER" ] && answered=$((answered+1))
  done
  echo "     $answered of $(echo $services | wc -w) services answered"
  "$app" set lyricsSources default >/dev/null
fi

if want youtube-music; then section "a YouTube Music profile, then the server's again (YouTube's to answer)"
  # The song's address is resolved through the core and its bytes come through media3's data source
  # from googlevideo: the glue Rust's tests stand in for. Opt-in, since YouTube may refuse on any day.
  server_widget_albums=$(widget_albums)
  check "the album widget has server albums before switching" test -n "$server_widget_albums"
  # YouTube Music is a plugin: installed from its package and turned on, its kind is offered.
  ytid=$(plugin_from_file youtube-music)
  check "the YouTube Music plugin installs and turns on" test -n "$ytid"
  "$app" login "kind:plugin:$ytid" >/dev/null
  check "the YouTube Music profile opens" wait_for kind "plugin:$ytid" 20
  yt_widget_albums=$(widget_albums)
  check "the album widget replaces the server's albums with YouTube albums" python3 - "$yt_widget_albums" <<'WIDGET'
import sys
ids = sys.argv[1].split(',')
assert ids and all(id.startswith('yt-') for id in ids), ids
WIDGET
  # Google's sign-in page is the provider's sign-in page, a WebView of the app's own; when it is done is
  # Rust's (the provider's sign_in_done, auth::signed_in_at).
  # That it opens, and that Cancel leaves the WebView no cookies, only a device can say.
  if [ "$(field signInOffer)" = True ]; then
    check "the profile just added offers Google's sign-in" tapnode text "Sign in to YouTube Music"
  else
    "$app" open sign-in >/dev/null
  fi
  check "the sign-in page opens" wait_for signInPage True 10
  check "and shows Google's sign-in form" wait_until 30 on_screen 'text="to continue to YouTube"'
  tapnode text Cancel
  check "Cancel closes it" wait_for signInPage False 10
  no_web_cookies() { [ "$("$app" webcookies)" = "cookies 0" ]; }
  check "and leaves the WebView no cookies" wait_until 10 no_web_cookies
  remember_settings autoMix autoFill lyricsOnline
  "$app" set autoMix true >/dev/null
  "$app" set autoFill false >/dev/null
  "$app" set lyricsOnline true >/dev/null
  watch_from_now
  "$app" play "search:daft punk one more time" >/dev/null
  check "a song found on YouTube Music plays through the AudioTrack" sounds 45
  at=$(field positionMs); written=$(field sinkBytes)
  check "its position advances" wait_for positionMs ">$((at + 3000))" 15
  check "and the sink takes its bytes" wait_for sinkBytes ">$written" 10
  echo "     playing '$(field title)' by '$(field artist)'"
  yt_first=$(field songId); yt_title=$(field title)
  "$app" do 'playnext search:daft punk harder better faster stronger' >/dev/null
  yt_analyzed() { [ "$(adb shell "run-as $pkg sqlite3 files/nori.db \"select count(*) from vocal_curve join track_analysis using(server,song_id) where song_id='$1' and length(curve)>0\"" | tr -d '\r')" -gt 0 ]; }
  check "YouTube playback bytes are analyzed" wait_until 120 yt_analyzed "$yt_first"
  "$app" open lyrics >/dev/null
  check "YouTube lyrics arrive" wait_for lyricLines '>0' 40
  "$app" do "lyrics song:$yt_first" >/dev/null
  yt_timing_checked() { [[ "$(field lyricsAnswerTiming)" == auto:* ]]; }
  check "the analyzed YouTube song gets a vocal timing check" wait_until 30 yt_timing_checked
  dur=$(field durationMs)
  "$app" do "seek $((dur - 30000))" >/dev/null
  check "the YouTube pair mixes" wait_for mixing True 45
  check "the next YouTube song takes over" wait_for title "!$yt_title" 30
  check "the incoming YouTube song has analysis and a vocal curve" wait_until 120 yt_analyzed "$(field songId)"
  check "and keeps sounding" sounds 10
  check "the incoming song plays beyond the transition" wait_for positionMs '>15000' 30
  "$app" do "undownload song:$yt_first" >/dev/null
  yt_saved() { [ "$(adb shell "run-as $pkg sqlite3 files/nori.db \"select count(*) from downloads where id='$yt_first' and done=1\"" | tr -d '\r')" = "$1" ]; }
  check "the YouTube fixture has no download" wait_until 10 yt_saved 0
  "$app" do "download song:$yt_first" >/dev/null
  check "a YouTube song downloads through its resolver" wait_until 60 yt_saved 1
  offline
  watch_from_now
  "$app" play "downloaded:$yt_first" >/dev/null
  check "the saved YouTube song is selected offline" wait_for songId "$yt_first" 10
  check "the downloaded YouTube song plays offline" sounds 20
  check "playback opens its downloaded files" waitfor_log 'yt-.*opens from its download' 10
  car_log=$(mktemp)
  check "Android Auto opens YouTube downloads offline" car_read downloads
  check "Android Auto lists the saved YouTube album" grep -Fq 'browse=true' "$car_log"
  "$app" do widgetfaces >/dev/null
  check "YouTube metadata renders in every widget face" waitfor_log "rendered six faces for $yt_first" 20
  online
  # Signed out, Home is YouTube's feed for everyone: its first song shelf plays from there.
  before=$(field title)
  "$app" play home >/dev/null
  check "the Home feed's first song plays" wait_for title "!$before" 30
  check "through the AudioTrack" sounds 45
  # Home's rows show no length; the core asks YouTube for them, so the seek bar has one and moves.
  home_song=$(field songId)
  check "the Home song has its length" wait_for durationMs '>0' 5
  check "and plays past 20 s" wait_for positionMs '>20000' 45
  check "without moving on" wait_for songId "$home_song" 1
  # The feed is read once for the visit (the core keeps it): scrolled away and back, YouTube is not
  # asked again, so the shelves show what they showed.
  "$app" open home >/dev/null
  home_top() { ui | grep -oE 'text="[^"]+"' | head -6; }
  home_shown() { local t; t=$(home_top); [ "$(printf '%s\n' "$t" | wc -l)" -ge 6 ] && first_home="$t"; }
  wait_until 30 home_shown
  size=$(adb shell wm size | grep -oE '[0-9]+x[0-9]+' | tail -1); w=${size%x*}; h=${size#*x}
  for _ in 1 2 3; do adb shell input swipe $((w / 2)) $((h * 3 / 4)) $((w / 2)) $((h / 4)) 200; done
  for _ in 1 2 3 4; do adb shell input swipe $((w / 2)) $((h / 4)) $((w / 2)) $((h * 3 / 4)) 200; done
  home_same() { [ "$(home_top)" = "$first_home" ]; }
  check "scrolled down and back up, Home's first shelf shows the same" wait_until 10 home_same
  "$app" open player >/dev/null
  check "the YouTube song menu opens" tapnode content-desc More
  check "its additional actions open" tapnode text More
  radio_seed=$(field songId); radio_from=$(field positionMs)
  check "YouTube offers song radio" tapnode text 'Start radio from this song'
  check "YouTube radio fills the queue" wait_for queue '>1' 30
  check "YouTube radio plays through the AudioTrack" sounds 20
  check "the song the radio started from plays on" wait_for positionMs ">$((radio_from + 30000))" 45
  check "without moving on" wait_for songId "$radio_seed" 1
  # Crossplay without a server: the terminal client on a YouTube profile, paired by its random secret.
  # Pairing, signing and the door's refusals are Rust's (crates/host tests/lan_jam.rs); here the phone
  # pairs through JNI, follows the door and hands its YouTube queue over.
  ytpeer="nori-e2e-ytpeer-$ANDROID_SERIAL"; ytdata="$here/../build/e2e/$ANDROID_SERIAL/ytpeer"
  python3 "$here/with-resource.py" build cargo build -j4 -p nori-cli >/dev/null
  tmux kill-session -t "$ytpeer" 2>/dev/null; rm -rf "$ytdata"; mkdir -p "$ytdata"
  python3 - "$ytdata/nori.db" <<'PY'
import sqlite3, sys
with sqlite3.connect(sys.argv[1]) as db:
    db.execute("CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY, value TEXT NOT NULL) WITHOUT ROWID")
    db.execute("INSERT OR REPLACE INTO settings VALUES('remoteControl', '{\"b\":true}')")
PY
  # The peer plays from the same plugin, installed in its own data folder and on.
  python3 - "$(plugin_package youtube-music)" "$ytdata/plugins" <<'PY'
import pathlib, sys, zipfile
root = pathlib.Path(sys.argv[2]); folder = root / "fm.nori.youtube-music"
zipfile.ZipFile(sys.argv[1]).extractall(folder)
(root / "index.json").write_text('{"plugin": [{"id": "fm.nori.youtube-music", "enabled": true}]}')
PY
  tmux new-session -d -s "$ytpeer" -x 160 -y 45 "$here/../target/debug/nori-cli --data $ytdata --provider fm.nori.youtube-music --no-images --no-mpris"
  for _ in $(seq 16); do tmux send-keys -t "$ytpeer" -- -; done
  yt_secret() { python3 -c 'import sqlite3,sys; r=sqlite3.connect(sys.argv[1]).execute("select value from app_kv where key=?",("remoteLanSecret",)).fetchone(); print(r[0]) if r else sys.exit(1)' "$ytdata/nori.db" 2>/dev/null; }
  check "the YouTube terminal peer makes its pairing secret" wait_until 20 yt_secret
  "$app" set remoteControl true >/dev/null
  check "the phone pairs with the terminal's code" test "$("$app" remote pair "nori://pair#k=$(yt_secret)")" = paired
  ytresolved=$(python3 "$here/mdns-door.py" "$ytdata/nori.db")
  check "the YouTube terminal peer announces its door" test -n "$ytresolved"
  ytname=$(printf '%s' "$ytresolved" | python3 -c 'import json,sys; print(json.load(sys.stdin)["name"])')
  ytdoor=$(printf '%s' "$ytresolved" | python3 -c 'import json,sys; print(json.load(sys.stdin)["door"])')
  ytpeer_id=$(printf '%s' "$ytdoor" | python3 -c 'import sys; print(dict(p.split("=",1) for p in sys.stdin.read().split("|",2)[2].split(";"))["id"])')
  "$app" remote watch on >/dev/null
  "$app" remote found "$ytdoor" >/dev/null
  check "the paired YouTube peer is listed" wait_until 15 bash -c "'$app' remote devices | grep -Fq '$ytname'"
  "$app" remote watch off >/dev/null
  "$app" remote pick "$ytpeer_id" >/dev/null
  check "the phone hands its YouTube queue to the terminal" wait_for playingOn "$ytname" 15
  check "this phone's output is let go" silent 10
  check "the terminal resolves and plays the YouTube song" wait_until 45 bash -c "'$app' remote devices | grep -Fq '$ytname (nearby): playing'"
  "$app" remote pick here >/dev/null
  check "\"This phone\" brings the YouTube queue back" wait_for playingOn "" 15
  check "and it sounds here" sounds 20
  check "resetting pairing answers" test "$("$app" remote reset-pairing)" = reset
  "$app" remote watch on >/dev/null
  "$app" remote found "$ytdoor" >/dev/null
  check "after a reset the terminal is no longer controllable" wait_until 15 bash -c "! '$app' remote devices | grep -Fq '$ytname'"
  "$app" remote watch off >/dev/null
  tmux kill-session -t "$ytpeer" 2>/dev/null
  "$app" set remoteControl false >/dev/null
  "$app" login "$APP_URL|$USER|$PASS" >/dev/null
  check "the server's profile opens again" wait_for server "$APP_URL" 30
  "$app" play "$SONG" >/dev/null
  check "and its songs play" sounds 20
  "$app" plugin reset >/dev/null
fi

if want notification; then section "the notification's seal and shuffle"
  # "notification <x>" sends the same session command the notification's button sends; "notification"
  # in the state is what the session last published to its controllers (the notification is one of them).
  # That the star reaches the server as Subsonic asks is client.rs a_heart_a_new_playlist_...
  "$app" play "song:$id" >/dev/null; sounds 20
  if [ "$NORI_E2E_SERVER" = local ]; then
    check "the notification fixture is the current song" wait_for title "Far Song Three" 10
  fi
  buttons=$(field notification); echo "     buttons: $buttons"
  check "the notification has a seal and a shuffle button" bash -c '[[ "'"$buttons"'" == *seal* && "'"$buttons"'" == *shuffle* ]]'
  was=$(field starred); flip=$([ "$was" = True ] && echo False || echo True)
  "$app" do "notification favourite" >/dev/null
  check "the notification's seal stars the song in the app ($was -> $flip)" wait_for starred "$flip" 10
  buttons=$(field notification)
  check "the notification's seal redraws ($buttons)" bash -c '[[ "'"$flip"'" == True && "'"$buttons"'" == *sealed* ]] || [[ "'"$flip"'" == False && "'"$buttons"'" != *sealed* ]]'
  "$app" do "notification favourite" >/dev/null; wait_for starred "$was" 10 >/dev/null   # put it back
  shuffle=$(field notification | grep -o 'shuffle_o[nf]*')
  "$app" do "notification shuffle" >/dev/null
  flipped=$([ "$shuffle" = shuffle_on ] && echo shuffle_off || echo shuffle_on)
  shuffled() { [ "$(field notification | grep -o 'shuffle_o[nf]*')" = "$1" ]; }
  check "the notification's shuffle toggles ($shuffle -> $flipped)" wait_until 5 shuffled "$flipped"
  "$app" do "notification shuffle" >/dev/null; wait_until 5 shuffled "$shuffle"   # put it back
  # The song's cover as the notification, the lock screen and the headphones open it (CarArtProvider):
  # drawn once, then read from the disk, with nothing in the app woken to draw it again.
  art="content://$pkg.carart/c/$(field cover)?s=800"
  adb shell "content read --uri '$art'" >/dev/null 2>&1
  drawn=$(field coversDrawn)
  kept_bytes=$(adb shell "content read --uri '$art' | wc -c" | tr -d '\r ')
  drawn_still() { [ "${kept_bytes:-0}" -gt 1000 ] && [ "$(field coversDrawn)" = "$drawn" ]; }
  check "the cover opened again is read from the disk, not drawn again ($kept_bytes bytes)" drawn_still
fi

if want car-art; then section "the car's mosaic when a cover is unavailable"
  covers=$(api getAlbumList2 "&type=newest&size=20" | python3 -c '
import json,sys
albums=json.load(sys.stdin)["subsonic-response"]["albumList2"].get("album",[])
ids=list(dict.fromkeys(a["coverArt"] for a in albums if a.get("coverArt")))[:4]
assert len(ids)==4, "four distinct fixture covers are needed"
print("|".join(ids))')
  art_log=$(mktemp)
  before=$(adb logcat -d -v brief -s noricar:I | wc -l | tr -d ' ')
  "$app" do "car art $covers" >/dev/null
  car_art_done() {
    adb logcat -d -v brief -s noricar:I | tail -n "+$((before + 1))" > "$art_log"
    grep -Fq "done art $covers" "$art_log"
  }
  check "the artwork provider's pictures decode" wait_until 20 car_art_done
  check "a missing cover keeps the other three in their quadrants" grep -Fq "kept=true" "$art_log"
  rm -f "$art_log"
fi

if want album-page; then section "the album page answers for its own queue"
  # The hero's pills answer for the queue the page started: Play becomes Pause while that queue sounds,
  # and a second press on Shuffle switches shuffle off where it stands. What each press means is
  # pages.rs the_big_buttons_answer_for_the_pages_own_queue; here the taps on the real screen. Picked: an
  # album whose songs all run past a minute, all the server's own, and not the one playing now.
  "$app" wake >/dev/null
  aid=""; playing_title=$(field title)
  if [ "$NORI_E2E_SERVER" = local ]; then
    aid=$(mix_album)
  else
    for a in $(api getAlbumList2 "&type=recent&size=25" | python3 -c "
import sys,json
for x in json.load(sys.stdin)['subsonic-response']['albumList2'].get('album',[]):
    if not x['id'].startswith('ext-') and x.get('songCount',0) >= 2: print(x['id'])"); do
      d=$(api getAlbum "&id=$a" | python3 -c "
import sys,json
s=json.load(sys.stdin)['subsonic-response']['album']['song']
t='''$playing_title'''
print(0 if any(x.get('title')==t or x['id'].startswith('ext-') or x.get('suffix')=='Remote' for x in s) else min(x.get('duration',0) for x in s))" 2>/dev/null)
      [ "${d:-0}" -ge 60 ] && { aid=$a; break; }
    done
  fi
  if [ -z "$aid" ]; then
    echo "     (no album of songs a minute or more long; nothing to tap at)"
  else
    "$app" do pause >/dev/null
    "$app" open "album/$aid" >/dev/null
    check "the pill reads Play before the album is played" wait_until 8 pill_is Play
    track=$(api getAlbum "&id=$aid" | python3 -c "import json,sys; print(json.load(sys.stdin)['subsonic-response']['album']['song'][0]['title'])")
    check "holding a track opens its song menu" "$here/ui-dump.sh" --long-click text "$track"
    check "the hold menu offers Play next" wait_until 8 on_screen 'text="Play next"'
    adb shell input keyevent 4
    "$app" play "album:$aid" >/dev/null; sounds 20
    check "the pill reads Pause while this album is what sounds" wait_until 8 pill_is Pause
    was=$(field shuffle)
    tapnode content-desc Shuffle
    check "shuffle starts this page's queue and lights ($was -> True)" wait_for shuffle True 8
    check "and the pill stays Pause over the queue it started" wait_until 5 pill_is Pause
    tapnode content-desc Shuffle
    check "a second press turns shuffle off on this queue" wait_for shuffle False 8
    tapnode text Pause
    check "the pill pauses playback" wait_for playing False 8
    check "and reads Play again" wait_until 5 pill_is Play
    pos=$(field positionMs)
    tapnode text Play
    check "Play picks the queue up where it stopped (from $pos ms)" wait_for positionMs ">$((pos + 500))" 10
  fi
fi

if want bridge; then section "the offline bridge"
  # A long library album, started while online and paused at once. Offline, a skip past what was fetched
  # ahead cannot play; with the bridge on, downloads play instead, and the album comes back with the
  # network. Which song is parked and where the album comes back is the core's (bridge.rs, and
  # a_bridge_is_started_and_undone_over_the_core_queue); here the network really going and coming.
  # Something has to be downloaded to stand in: the smoke's download, or this one.
  if [ "$(field downloaded)" = 0 ]; then
    "$app" do "download song:$id" >/dev/null; wait_for downloaded ">0" 60 >/dev/null
  fi
  "$app" set bridgeOffline true >/dev/null
  # Nothing of the album may already be on the phone: a song in the stream cache or downloaded plays
  # offline, rightly, and then there is nothing to bridge. What the phone holds is read from its database.
  "$app" set clearStreamCache true >/dev/null
  dl=$(mktemp -d)
  for f in nori.db nori.db-wal nori.db-shm; do adb exec-out run-as "$pkg" cat files/$f > "$dl/$f" 2>/dev/null; done
  python3 -c "
import sqlite3
c=sqlite3.connect('$dl/nori.db')
print('\n'.join(r[0] for r in c.execute('select id from downloads')))" > "$dl/held" 2>/dev/null
  if [ "$NORI_E2E_SERVER" = local ]; then candidates=$(mix_album); else
    candidates=$(api getAlbumList2 "&type=random&size=100" | python3 -c "
import sys,json
for a in json.load(sys.stdin)['subsonic-response']['albumList2'].get('album',[]):
    if not a['id'].startswith('ext-') and a.get('songCount',0) >= 10: print(a['id'])"); fi
  bid=$(for a in $candidates; do api getAlbum "&id=$a" | python3 -c "
import sys,json
held=set(open('$dl/held').read().split())
s=json.load(sys.stdin)['subsonic-response']['album']['song']
own=all(not x['id'].startswith('ext-') and x.get('suffix')!='Remote' for x in s)
print('$a' if own and len(s)>=10 and not any(x['id'] in held for x in s) else '')"; done | grep . | head -1)
  rm -rf "$dl"
  if [ -n "$bid" ]; then
    "$app" play "album:$bid" >/dev/null; wait_for playing True 20 >/dev/null; "$app" do pause >/dev/null
    offline
    for _ in 1 2 3 4 5 6; do "$app" do next >/dev/null; done
    check "downloads stand in while the server is out of reach" wait_for bridging True 30
    check "and they play" sounds 20
    online
    check "the album comes back with the network" wait_for bridging False 30
    "$app" do pause >/dev/null
  else
    echo "     (no library album of ten songs with nothing of it downloaded)"
  fi
  "$app" set bridgeOffline false >/dev/null
fi

if want download-notification; then section "the download queue from its notification"
  # The notification's tap is this intent; the app is already running, so it arrives as a new intent.
  "$app" open home >/dev/null
  adb shell am start -a dev.nori.music.OPEN_DOWNLOADS -n "$pkg/dev.nori.music.app.MainActivity" >/dev/null 2>&1
  check "tapping the download notification opens the queue" wait_for route downloads 8
fi

if want mobile-data; then section "a download asked for on mobile data waits for a yes"
  # Whether the phone is on mobile data is Android's (ConnectivityManager); asking first is the app's.
  check "the emulator starts on Wi-Fi" wait_until 30 active_network WIFI
  dl=$(song_id "Long Track 02")
  saved() { [ "$(adb shell "run-as $pkg sqlite3 files/nori.db \"select count(*) from downloads where id='$dl' and done=1\"" | tr -d '\r')" = "$1" ]; }
  "$app" do "undownload song:$dl" >/dev/null
  check "the song starts with no download" wait_until 10 saved 0
  "$app" open home >/dev/null
  "$app" do "download song:$dl" >/dev/null
  check "on Wi-Fi it downloads without asking" wait_until 60 saved 1
  check "and nothing asks" off_screen 'text="Download on mobile data?"'
  "$app" do "undownload song:$dl" >/dev/null
  wait_until 10 saved 0
  adb shell svc wifi disable; adb shell svc data enable
  check "the emulator is on mobile data" wait_until 30 active_network MOBILE
  "$app" do "download song:$dl" >/dev/null
  check "on mobile data it asks first" wait_until 10 on_screen 'text="Download on mobile data?"'
  check "and has not downloaded" saved 0
  tapnode text "Download"
  check "a yes downloads it" wait_until 60 saved 1
  online
  check "and Wi-Fi is back" active_network WIFI
  "$app" do "undownload song:$dl" >/dev/null
fi

if want downloads; then section "downloads run side by side through media3 and survive a force stop"
  # How the batch is counted, its speed and time left are transfers.rs; here media3 running them.
  # A library album of at least six songs with nothing of it downloaded yet (every run downloads one).
  held=$(adb shell "run-as $pkg sqlite3 files/nori.db \"select distinct json_extract(json,'\$.albumId') from items where kind=2 and id in (select id from downloads)\"" 2>/dev/null | tr -d '\r')
  if [ "$NORI_E2E_SERVER" = local ]; then
    # The generated albums: a few hundred to take from before any is used twice. Never the Long Album,
    # which the bridge needs with nothing of it downloaded.
    aids=$(api getAlbumList2 "&type=alphabeticalByName&size=500" | python3 -c "
import sys,json
for a in json.load(sys.stdin)['subsonic-response']['albumList2'].get('album',[]):
    if a.get('songCount',0) >= 3: print(a['id'])" | grep -vxF -e "${held:-none}" -e "$(mix_album)" | head -12 | tr '\n' ' ')
  else
    aids=$(api getAlbumList2 "&type=random&size=100" | python3 -c "
import sys,json
for a in json.load(sys.stdin)['subsonic-response']['albumList2'].get('album',[]):
    if not a['id'].startswith('ext-') and a.get('songCount',0) >= 6: print(a['id'])" | grep -vxF -e "${held:-none}" | while read -r a; do
      api getAlbum "&id=$a" | python3 -c "
import sys,json
s=json.load(sys.stdin)['subsonic-response']['album']['song']
print('$a' if all(not x['id'].startswith('ext-') and x.get('suffix')!='Remote' for x in s) else '')"; done | grep . | head -12 | tr '\n' ' ')
  fi
  if [ -n "$aids" ]; then
    before=$(field downloaded)
    # An album already downloaded has nothing left to fetch, so ask each candidate until one has work.
    active=0
    for a in $aids; do
      "$app" do "download album:$a" >/dev/null
      wait_for dlActive ">1" 5 2>/dev/null && { active=$WAITED; break; }
    done
    check "several songs download at once ($active)" test "${active:-0}" -ge 2
    # Under its own id: it used to share the playback notification's (1001) and replace it. 2001 while
    # songs are coming, 2002 once they are done (the local server can finish before this looks).
    notifs=$(adb shell dumpsys notification --noredact 2>/dev/null | grep -o "$pkg|[0-9]*" | sort -u | tr '\n' ' ')
    check "the download notification posts under its own id ($notifs)" bash -c "[[ '$notifs' == *'|2001'* || '$notifs' == *'|2002'* ]]"
    adb shell am force-stop "$pkg" >/dev/null 2>&1; app_up >/dev/null
    resumed() { local v; v=$(fields downloading dlActive | tr '\n' ' '); set -- $v; [ "${1:-1}" = 0 ] || [ "${2:-0}" -gt 0 ]; }
    check "after a force stop the queue picks up again" wait_until 15 resumed
    check "the interrupted album finishes (was $before downloaded)" wait_for downloading 0 180
  else
    echo "     no library-only album with work left found; skipped"
  fi
fi

if want download-albums; then section "downloaded albums on the phone and through the car's media browser"
  remember_settings autoMixBetterBeats
  "$app" set autoMixBetterBeats false >/dev/null
  album_id=$(mix_album); first_id=$(song_id "Long Track 01"); third_id=$(song_id "Long Track 03")
  car_log=$(mktemp)
  "$app" do "undownload album:$album_id" >/dev/null
  removed_album() { car_read downloads && ! grep -Fq "downloaded:$album_id |" "$car_log"; }
  check "the fixture album starts without saved tracks" wait_until 20 removed_album
  before=$(field downloaded)
  "$app" do "download song:$first_id" >/dev/null
  check "the first fixture track finishes through media3" wait_for downloaded ">$before" 60
  "$app" do "download song:$third_id" >/dev/null
  check "the second fixture track finishes through media3" wait_for downloaded ">$((before + 1))" 60
  check "the car can read its downloads tab" car_read downloads
  check "the car lists the saved album as a folder" grep -Fq "downloaded:$album_id | Long Album" "$car_log"
  check "the car opens the saved tracks" car_read "downloaded:$album_id"
  check "only downloaded tracks appear in the album" python3 - "$car_log" "$first_id" "$third_id" <<'CAR'
import re,sys
text=open(sys.argv[1]).read()
ids=re.findall(r': in\|downloaded:[^|]+\|([^ ]+) \|',text)
assert ids == sys.argv[2:], ids
CAR
  # A tap on the song already current resumes it as it was and opens the player (ActionsViewModel.tap);
  # an earlier section can leave this album's first track there, so another song is current.
  "$app" play "$OTHER" >/dev/null; wait_for playing True 20 >/dev/null; "$app" do pause >/dev/null
  "$app" open library >/dev/null
  check "the library's Downloads button opens its page" tapnode content-desc Downloads
  check "the phone lists the saved album" wait_until 10 on_screen 'text="Long Album"'
  check "the phone opens the downloaded album" tapnode text "Long Album"
  check "the album uses its downloaded page" wait_for route "downloaded/{id}" 8
  check "its saved tracks appear on the phone" wait_until 10 on_screen 'text="Long Track 01"'
  # The song an earlier section left playing is named in the now playing bar; only a row of the page counts.
  unsaved_absent() {
    local rows; rows=$(ui | grep -o 'text="Long Track 02"' | wc -l)
    [ "$(field title)" = "Long Track 02" ] && rows=$((rows - 1))
    [ "$rows" -le 0 ]
  }
  check "unsaved tracks are absent on the phone" unsaved_absent
  offline
  check "a saved album track can be tapped offline" tapnode text "Long Track 01"
  check "a saved album track plays offline" sounds 10
  online
  was_starred=$(starred_on_server "$first_id")
  [ "$was_starred" = True ] || "$app" do "star song:$first_id" >/dev/null
  check "the car's Favorites tab loads" car_read starred
  check "the favorite crosses the native media browser" grep -Fq "in|starred|$first_id |" "$car_log"
  [ "$was_starred" = True ] || "$app" do "star song:$first_id" >/dev/null
  # The downloaded album's page is still open: removing its tracks empties it while its ⋯ menu fades out.
  check "the downloaded album's page is still open" wait_for route "downloaded/{id}" 8
  app_pid=$(adb shell pidof "$pkg" | tr -d '\r')
  "$app" do "undownload album:$album_id" >/dev/null
  check "removing the saved tracks removes the album folder" wait_until 20 removed_album
  check "emptying the open downloaded album does not crash the app" test "$(adb shell pidof "$pkg" | tr -d '\r')" = "$app_pid"
  rm -f "$car_log"
fi

if want foryou; then section "for you: favourites and mixes open as pages"
  # What a mix page shows, read from the screen: "<count>|<title>@<x>,<y>|..." for the song count in its
  # caption and every fully visible row title (rows sit below the Play pill and above the mini player).
  # Which songs a mix holds, and that it stays the same when opened again, is mixes.rs.
  page() {
    ui | python3 -c "
import sys,re
nodes=[(t,d,*map(int,b)) for t,d,b in ((m.group(1),m.group(2),re.findall(r'\d+',m.group(3))) for m in re.finditer(r'text=\"([^\"]*)\"[^>]*content-desc=\"([^\"]*)\"[^>]*bounds=\"([^\"]*)\"',sys.stdin.read()))]
count=next((int(m.group(1)) for t,*_ in nodes for m in [re.match(r'(\d+) songs? ',t)] if m),0)
bar=next((y1 for t,d,x1,y1,x2,y2 in nodes if d=='Now playing bar'),10**6)
play=next((y2 for t,d,x1,y1,x2,y2 in nodes if t=='Play'),0)
rows=[n for n in nodes if n[0] and n[3]>=play and n[5]<=bar and 150<n[2]<260]
titles=[n for i,n in enumerate(rows) if i%2==0]
print('|'.join([str(count)]+['%s@%d,%d'%(t,(x1+x2)//2,(y1+y2)//2) for t,d,x1,y1,x2,y2 in titles]))"
  }
  count_is() { [ "$(page | cut -d'|' -f1)" = "$1" ]; }
  starred_count() { api getStarred2 | python3 -c "
import sys,json
d=json.load(sys.stdin)['subsonic-response'].get('starred2',{})
print(sum(1 for s in d.get('song',[]) if not s.get('isExternal') and not s['id'].startswith(('ext-','pl-'))))"; }
  "$app" open mix/favourites >/dev/null
  check "the favourites tile opens its page" wait_for route "mix/{id}" 8
  server=$(starred_count)
  check "it lists the songs the server has starred (server $server)" wait_until 8 count_is "$server"
  star_delta=$([ "$(starred_on_server "$id")" = True ] && echo -1 || echo 1)
  "$app" do "star song:$id" >/dev/null
  check "toggling a favorite updates the open page" wait_until 10 count_is "$((server + star_delta))"
  "$app" do "star song:$id" >/dev/null   # put it back
  check "restoring the favorite restores the open page" wait_until 10 count_is "$server"
  if [ "$NORI_E2E_SERVER" = local ]; then
    # A fresh fixture profile needs album rows in its local index for Discover.
    "$app" open "album/$(mix_album)" >/dev/null
    check "the fixture album loads" wait_until 15 on_screen 'text="Long Track 01"'
  fi
  "$app" open mix/discover >/dev/null
  wait_until 8 bash -c "'$app' state | grep -q 'mix/{id}'"
  first=""
  page_ready() { first=$(page) || return 1; [ -n "$(echo "$first" | cut -d'|' -f4)" ]; }
  wait_until 15 page_ready
  # What you see is what plays: a tap on the third row starts the whole mix at that row.
  n=$(echo "$first" | cut -d'|' -f1); third=$(echo "$first" | cut -d'|' -f4)
  if [ -n "$third" ]; then
    xy=${third##*@}; adb shell input tap "${xy%,*}" "${xy#*,}"
    tapped_queue() {
      "$app" state | python3 -c '
import json,sys
s=json.load(sys.stdin)
sys.exit(0 if s.get("title")==sys.argv[1] and s.get("index")==2 and s.get("queue")==int(sys.argv[2]) else 1)
' "${third%@*}" "$n"
    }
    check "tapping a row starts the whole mix at that song (${third%@*}, page $n)" wait_until 10 tapped_queue
    "$app" do pause >/dev/null
  else
    check "the mix has songs to play" false
  fi
fi

if want dac; then section "a USB DAC, faked"
  # A DAC cannot be plugged into an emulator, so the app is pointed at a mock one (ActionsViewModel,
  # "dac"). Which mode a DAC gets and why one cannot be fed is dac.rs; here what the platform does: offload
  # stands down when the device appears, and the track is opened in the DAC's own format.
  "$app" set autoMix false >/dev/null; "$app" set crossfadeSec 0 >/dev/null
  "$app" set headphoneSound OFF >/dev/null; "$app" set offload true >/dev/null; "$app" set eq false >/dev/null
  "$app" do "dac off" >/dev/null
  "$app" play "$PLAIN" >/dev/null; sounds 20
  check "offload is asked for on the phone's own output" wait_for offloadWanted True 10
  "$app" do "dac Mock DAC@44100/16,96000/24" >/dev/null
  check "offload stands down when a USB device appears" wait_for offloadWanted False 10
  check "the DAC is seen" wait_for dac "Mock DAC" 5
  check "audio keeps flowing to the DAC" bursts_continue
  "$app" set bitPerfect true >/dev/null; "$app" play "$PLAIN" >/dev/null; sounds 20
  check "bit-perfect engages on a mode the sink can write" wait_for bitPerfect True 10
  check "and says what the track was opened with" test -n "$(field dacTrack)"
  "$app" do "dac off" >/dev/null; "$app" set bitPerfect false >/dev/null
fi

if want device-sound; then section "a sound per output device"
  # The service switches the sound when the output changes, with no screen involved. A fake DAC stands in
  # for the device; "eq" in the state is the equalizer switch, which each device's sound sets. What a
  # device gets (flat, a profile, AutoEQ offered, applied or undone) is profiles.rs and device.rs; here the
  # platform's output events reaching it, both ways.
  dev="USB: Nori Check DAC"
  clean() {
    "$app" do "dac off" >/dev/null; "$app" set autoEqAuto false >/dev/null
    "$app" set deleteProfile "nori check" >/dev/null; "$app" set forgetDevice "$dev" >/dev/null
  }
  clean   # a run that stopped half-way must not decide this one
  "$app" set eq false >/dev/null
  "$app" play "$PLAIN" >/dev/null; sounds 20
  "$app" set eq true >/dev/null; "$app" set saveProfile "nori check" >/dev/null; "$app" set eq false >/dev/null
  "$app" set deviceSound "$dev=profile:nori check" >/dev/null
  "$app" do "dac Nori Check DAC@44100/16" >/dev/null
  on_dev() { [ "$(fields output eq | tr '\n' '/')" = "$dev/True/" ]; }
  check "a device with a profile gets it on connect" wait_until 10 on_dev
  "$app" do "dac off" >/dev/null
  check "and the sound from before comes back without it" wait_for eq False 10
  clean   # nothing of the check stays in the device list or the profiles
fi

if want sheets; then section "a sheet over the sideways player, the shade pulled down and put back"
  # The focused window decides whether the status bar shows. A sheet is a window of its own: unless it
  # asks for what the app asked for, the bar comes back over the sideways player, and goes and comes as
  # the shade takes the focus and gives it back, moving the sheet with the insets.
  status_bar_away() { adb shell dumpsys window | grep -m1 'InsetsSource.*type=statusBars' | grep -q 'visible=false'; }
  sheets_up() { [ "$(ui | grep -o 'text="Playing on"' | wc -l | tr -d ' ')" = "$1" ]; }
  sheet_at() { ui | grep -o 'text="Playing on"[^>]*bounds="[^"]*"' | grep -o 'bounds="[^"]*"'; }
  "$app" set hideStatusBar SIDEWAYS >/dev/null; "$app" set remoteControl true >/dev/null
  "$app" play "$PLAIN" >/dev/null; sounds 20
  "$app" open player >/dev/null
  adb shell settings put system accelerometer_rotation 0; adb shell settings put system user_rotation 1
  check "the sideways player puts the status bar away" wait_until 10 status_bar_away
  "$app" open devices >/dev/null
  check "Playing on comes up" wait_until 10 on_screen 'text="Playing on"'
  before=$(sheet_at)
  check "and leaves the status bar away" status_bar_away
  adb shell cmd statusbar expand-settings; wait_until 5 sheets_up 0
  adb shell cmd statusbar collapse
  check "Playing on is there once after the shade" wait_until 10 sheets_up 1
  check "the status bar stays away after the shade" status_bar_away
  check "the sheet has not moved" test "$(sheet_at)" = "$before"
  adb shell input keyevent KEYCODE_BACK
  check "back closes the sheet" wait_until 10 off_screen 'text="Playing on"'
  adb shell settings put system user_rotation 0; adb shell settings put system accelerometer_rotation 1
  "$app" set remoteControl false >/dev/null
fi

if want queue-panel; then section "the song playing, tapped in the queue"
  "$app" play "$PLAIN" >/dev/null; "$app" do "enqueue $SONG" >/dev/null
  check "the song plays a few seconds" wait_for positionMs '>3000' 15
  "$app" do pause >/dev/null; check "paused" wait_for playing False 5
  at=$(field positionMs)
  "$app" open queue >/dev/null
  check "the queue opens" wait_until 10 on_screen 'content-desc="Shuffle"'
  check "it says Playing next once" test "$(ui | grep -oi 'text="Playing next"' | wc -l | tr -d ' ')" = 1
  tapnode text "$(field title)"
  check "goes back to the player" wait_until 10 off_screen 'content-desc="Shuffle"'
  check "and stays where it was" test "$(field positionMs)" = "$at"
fi

remote_checks() {
  section "playing on another device: the terminal client on this computer"
  # Who plays, the queue, the place, the volume and a transfer's order are the core's (crates/core
  # tests/remote.rs). Here: the media session handed to the other device (a remote volume the keys move),
  # this phone's own output let go while it plays there, and the music coming back.
  if [ "$NORI_E2E_SERVER" != local ]; then
    echo "  NOTE  needs the local server: the terminal client signs in to it as the other device"
  else
    peer="nori-e2e-peer-$ANDROID_SERIAL"; data="$here/../build/e2e/$ANDROID_SERIAL/peer"; cli="$here/../target/debug/nori-cli"
    python3 "$here/with-resource.py" build cargo build -j4 -p nori-cli >/dev/null || return 1
    tmux kill-session -t "$peer" 2>/dev/null; mkdir -p "$data"
    # Seed the peer's setting before startup; no screen navigation or stale selection.
    python3 - "$data/nori.db" <<'PY'
import sqlite3, sys
with sqlite3.connect(sys.argv[1]) as db:
    db.execute("CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY, value TEXT NOT NULL) WITHOUT ROWID")
    db.executemany("INSERT OR REPLACE INTO settings VALUES(?, ?)", [
        ("remoteControl", '{"b":true}'), ("crossfadeSec", '{"i":4}'),
    ])
PY
    tmux new-session -d -s "$peer" -x 160 -y 45 "$cli --data $data --url http://localhost:4533 --user $USER --password $PASS --no-images --no-mpris"
    screen() { tmux capture-pane -p -t "$peer"; }
    keys() { tmux send-keys -t "$peer" -- "$@"; }
    check "the CLI peer starts" wait_until 15 bash -c "tmux capture-pane -p -t $peer | grep -q 'localhost:4533'" || return 1
    for _ in $(seq 16); do keys -; done   # quiet on the peer's speakers
    # Its door as mDNS has it; the emulator hears no multicast from here, so the app is handed it.
    resolved=$(python3 "$here/mdns-door.py" "$data/nori.db") || { check "the CLI peer announces its door" false; return 1; }
    name=$(printf '%s' "$resolved" | python3 -c 'import json,sys; print(json.load(sys.stdin)["name"])')
    door=$(printf '%s' "$resolved" | python3 -c 'import json,sys; print(json.load(sys.stdin)["door"])')
    peer_id=$(printf '%s' "$door" | python3 -c 'import sys; print(dict(p.split("=",1) for p in sys.stdin.read().split("|",2)[2].split(";"))["id"])')
    echo "     the peer's door: $door"
    "$app" set remoteControl true >/dev/null
    "$app" play "album:$(mix_album)" >/dev/null; sounds 20
    check "the transfer starts with the album's first song" wait_for title "Long Track 01" 10
    "$app" open player >/dev/null; "$app" remote watch on >/dev/null
    "$app" remote found "$door" >/dev/null
    check "the peer is listed" wait_until 15 bash -c "'$app' remote devices | grep -Fq '$name'" || return 1
    "$app" remote watch off >/dev/null
    "$app" remote pick "$peer_id" >/dev/null
    check "the player shows the peer playing" wait_for playingOn "$name" 15 || return 1
    remote_session() { adb shell dumpsys media_session | grep -q "volumeType=REMOTE"; }
    check "the media session is the peer's (a remote volume)" wait_until 10 remote_session
    check "this phone's output is let go" silent 10
    check "the peer's engine is playing before the phone closes" wait_until 10 bash -c "'$app' remote devices | grep -Fq '$name (nearby): playing'" || return 1
    adb shell am force-stop "$pkg"
    "$app" launch >/dev/null
    "$app" open player >/dev/null
    remote_started() {
      local listed; listed=$("$app" remote devices) || return 1
      [[ "$listed" == none || "$listed" == *": playing "* || "$listed" == *": paused "* ]]
    }
    check "the foreground starts its remote controller" wait_until 10 remote_started || return 1
    "$app" remote found "$door" >/dev/null
    check "reopening follows the peer without the device picker" wait_for playingOn "$name" 15 || return 1
    check "the remote media session returns after reopening" wait_until 10 remote_session
    check "reopening does not play the saved queue on this phone" silent 10
    # The cells lit in the terminal's volume bar, after its word.
    peer_volume() { screen | perl -CS -Mutf8 -ne 'if (/Volume (━*)/) { $n = length $1 } END { print $n // "" }'; }
    before=$(peer_volume)
    check "the peer shows its volume" test -n "$before"
    adb shell dumpsys media_router > "$here/../build/e2e/$ANDROID_SERIAL/native-routes.txt"
    adb shell input keyevent KEYCODE_VOLUME_UP; adb shell input keyevent KEYCODE_VOLUME_UP
    louder() { [ "$(peer_volume)" -gt "$before" ]; }
    check "the volume keys turn the peer up (from $before)" wait_until 10 louder
    for track in 02 03 04; do
      "$app" do next >/dev/null
      check "the remote queue reaches song $track" wait_for title "Long Track $track" 10
    done
    check "remote skips keep this phone silent" silent 10
    for _ in $(seq 16); do keys -; done
    # Songs of one album in order never mix: shuffled, the next one does.
    "$app" do shuffle >/dev/null
    "$app" do "seek $(( $(field durationMs) - 4000 ))" >/dev/null
    check "remote playback reports the host's active mix" wait_for displayMixing True 10
    check "the controlling phone shows Mixing" wait_until 5 on_screen 'text="MIXING"'
    check "the controlling phone keeps its own output silent during the mix" wait_for mixing False 5
    "$app" play "search:Long Track 04" >/dev/null
    check "a new remote play clears the mix indicator" wait_for displayMixing False 10
    check "the peer returns to the selected song" wait_for title "Long Track 04" 10
    # The notification's seal stands for the peer's song: its star is the one the peer's queue holds.
    was=$(field starred); flip=$([ "$was" = True ] && echo False || echo True)
    "$app" do "notification favourite" >/dev/null
    check "the notification's seal stars the peer's song ($was -> $flip)" wait_for starred "$flip" 10
    peer_seal() { local b; b=$(field notification); [[ "$flip" == True && "$b" == *sealed* ]] || [[ "$flip" == False && "$b" != *sealed* ]]; }
    check "the notification's seal redraws for the peer's song" wait_until 10 peer_seal
    "$app" do "notification favourite" >/dev/null; wait_for starred "$was" 10 >/dev/null   # put it back
    adb shell am broadcast -a com.android.systemui.action.LAUNCH_MEDIA_OUTPUT_DIALOG -p com.android.systemui --es package_name "$pkg" >/dev/null
    check "the native output picker opens" wait_until 10 on_screen 'package="com.android.systemui"'
    check "the native picker offers this phone" wait_until 10 on_screen 'text="This phone"'
    check "the native picker can return playback here" tapnode text "This phone"
    check "native transfer returns the session to the phone" wait_for playingOn "" 15
    check "the transfer keeps the peer's latest song" wait_for title "Long Track 04" 10
    if on_screen 'text="Done"'; then tapnode text Done; fi
    "$app" remote pick here >/dev/null
    check "\"This phone\" brings it back" wait_for playingOn "" 15
    check "and it sounds here" sounds 15
    check "the peer paused" wait_until 10 bash -c "tmux capture-pane -p -t $peer | grep -q '▶'"
    "$app" remote pick "$peer_id" >/dev/null
    check "the phone can hand playback back to the peer" wait_for playingOn "$name" 10
    check "the phone releases its output again" silent 10
    check "the peer is playing before it closes" wait_until 10 bash -c "'$app' remote devices | grep -Fq '$name (nearby): playing'"
    tmux kill-session -t "$peer"
    "$app" remote pick here >/dev/null
    check "a closed peer immediately releases the phone controls" wait_for playingOn "" 3
    "$app" play "$PLAIN" >/dev/null
    check "local playback works after the peer closes" sounds 10
    "$app" set remoteControl false >/dev/null
  fi
}
if want remote; then
  failures_before=$fail
  remote_checks || { [ "$fail" -gt "$failures_before" ] || check "remote setup succeeds" false; }
  "$app" remote pick here >/dev/null
  "$app" set remoteControl false >/dev/null
  [ -z "${peer:-}" ] || tmux kill-session -t "$peer" 2>/dev/null
fi

jam_checks() {
  section "a jam hosted here: started from the devices sheet, guests ask, the host decides by tapping"
  # The jam's roles, requests, who added what, and that a provider's song is not looked up before it is
  # accepted are the core's (crates/core tests/remote.rs). Here: the jam in the player, its queue and the
  # devices sheet, the invite's link, requests arriving live and decided by tapping, and the accepted song
  # playing, against octo-fiesta's real relay (NORI_E2E_JAM, the local one on 5274) with two guests on this
  # Mac (tools/jam-guest.py).
  jam_server=${NORI_E2E_JAM:-http://localhost:5274}
  if [ "$NORI_E2E_SERVER" != local ] || ! curl -sf "$jam_server/rest/noriRemote.poll?u=admin&p=admin&v=1.16.1&c=e2e&f=json&dev=e2e-probe" | grep -q seq; then
    if [ "$NORI_E2E_SERVER" = local ]; then
      check "octo-fiesta relay is available at $jam_server" false
      return 1
    fi
    echo "  NOTE  needs the local server, and octo-fiesta with the relay in front of it at $jam_server"
  else
    check "the relay serves its invite page" bash -c "curl -fsS '$jam_server/nori/jam' | grep -q 'Open in nori'" || return 1
    app_jam=$(echo "$jam_server" | sed 's#localhost#10.0.2.2#')
    start_jam() {
      tapnode text "Start a jam" || return 1
      wait_until 10 on_screen 'text="Your jam"'
    }
    lib_song=$(song_id "Far Song Two"); first=$(song_id "Long Track 04")
    # A provider's song for Dee to ask for: only its id and words travel, nothing streams it.
    provider='{"id":"ext-e2e-refused","title":"Refused provider request","artist":"Nori E2E","duration":170,"isExternal":true}'
    "$app" login "$app_jam|admin|admin" >/dev/null; wait_for server "$app_jam" 30 >/dev/null
    "$app" set remoteControl true >/dev/null
    "$app" play "song:$first" >/dev/null; sounds 20
    "$app" open devices >/dev/null
    check "the devices sheet offers to start a jam" wait_until 10 on_screen 'text="Start a jam"' || return 1
    check "starting the jam expands the existing devices sheet" start_jam || return 1
    check "the devices sheet shows the hosted jam" wait_until 10 on_screen 'text="Your jam"' || return 1
    check "nobody listens yet" on_screen 'text="Jam · no one yet"'
    check "the devices sheet offers the invite" wait_until 10 on_screen 'text="Invite"'
    tapnode text Invite
    check "Invite shows the code and the link" wait_until 10 on_screen 'text="Copy link"'
    ui > "$here/../build/e2e/$ANDROID_SERIAL/jam-invite.xml"
    link=$(cat "$here/../build/e2e/$ANDROID_SERIAL/jam-invite.xml" | grep -oE 'text="https?://[^"]*/nori/jam#[^"]*"' | head -1 | cut -d'"' -f2 | sed 's/&amp;/\&/g')
    tapnode content-desc Back
    check "Invite returns to the existing room drawer" wait_until 10 on_screen 'text="Your jam"'
    check "the invite content is replaced" off_screen 'text="Copy link"'
    tapnode text Invite
    check "Invite opens again in the same drawer" wait_until 10 on_screen 'text="Copy link"'
    adb shell input keyevent KEYCODE_BACK
    check "system back returns to the room drawer" wait_until 10 on_screen 'text="Your jam"'
    adb shell input keyevent KEYCODE_BACK
    if on_screen 'content-desc="Close sheet"'; then tapnode content-desc "Close sheet"; fi
    # Its own invite opened here is refused: this phone stays the host, on its own profile.
    check "the invite reaches nori" open_invite "$link" app || return 1
    check "its own invite is refused" wait_until 10 on_screen 'text="That’s your own Jam"' || return 1
    check "and it still hosts" bash -c "'$app' remote view | grep -q 'hosting=true'"
    check "on its own profile" wait_for server "$app_jam" 5
    "$app" open devices >/dev/null
    tapnode content-desc Settings
    check "room settings replace the drawer" wait_until 10 on_screen 'text="Allow remote songs"'
    tapnode text "Allow remote songs"
    tapnode content-desc Back
    check "settings return to the existing room drawer" wait_until 10 on_screen 'text="Your jam"'
    adb shell input keyevent KEYCODE_BACK
    "$app" open queue >/dev/null
    guests="$here/../build/e2e/$ANDROID_SERIAL/jam"; mkdir -p "$guests"
    python3 "$here/jam-guest.py" "$link" Gus "{\"id\":\"$lib_song\",\"title\":\"Far Song Two\",\"artist\":\"Nori E2E Two\",\"duration\":170}" > "$guests/gus.log" 2>&1 & gus=$!
    check "Gus's request comes in by itself" wait_until 20 on_screen 'text="Asked by Gus"' || return 1
    python3 "$here/jam-guest.py" "$link" Dee "$provider" > "$guests/dee.log" 2>&1 & dee=$!
    check "and Dee's under it" wait_until 20 on_screen 'text="Asked by Dee"' || return 1
    check "the provider's song says accepting downloads it" on_screen 'text="Downloaded to your server if accepted"'
    "$app" open devices >/dev/null
    check "the devices sheet counts both" wait_until 10 on_screen 'text="Jam · 2 listening"'
    adb shell input keyevent KEYCODE_BACK
    check "the devices sheet closes before deciding requests" wait_until 10 off_screen 'content-desc="Close sheet"'
    check "Dee's Refuse button can be tapped" tapnode content-desc Refuse 2 || return 1
    check "Refuse takes Dee's request away" wait_until 10 off_screen 'text="Asked by Dee"'
    tapnode content-desc Accept
    queued() { [[ "$(field upNext)" == *"$lib_song"* ]]; }
    check "Accept queues Gus's song" wait_until 15 queued
    check "nothing of the provider's was queued" bash -c "! '$app' state | grep -q 'ext-'"
    check "its row says Gus added it" wait_until 10 on_screen 'content-desc="Added by Gus"'
    check "Gus sees it in the host's queue, by him" wait_until 15 grep -q "Far Song Two by Gus" "$guests/gus.log"
    tapnode text "Far Song Two"
    check "and it plays" wait_for title "Far Song Two" 15
    "$app" open devices >/dev/null
    tapnode content-desc End
    adb shell input keyevent KEYCODE_BACK
    check "End Jam ends it for the guests" wait_until 15 grep -q "the jam is over" "$guests/gus.log"
    check "and here" wait_until 10 bash -c "'$app' remote view | grep -q 'no jam'"
    kill $gus $dee 2>/dev/null
    # Its own invite to the jam it ended changes nothing, and a new one starts at once.
    check "the ended invite reaches nori" open_invite "$link" app || return 1
    check "its ended jam's invite says so" wait_until 10 on_screen 'text="This Jam has ended"'
    check "on its own profile, in no jam" bash -c "'$app' remote view | grep -q 'no jam'"
    check "no jam under the song" off_screen 'text="Jam · '
    "$app" open devices >/dev/null
    check "the devices sheet offers a jam again" wait_until 10 on_screen 'text="Start a jam"'
    check "restarting the jam expands the existing devices sheet" start_jam || return 1
    check "and it starts at once" wait_until 5 on_screen 'text="Your jam"'
    tapnode content-desc End
    adb shell input keyevent KEYCODE_BACK
    check "and ends" wait_until 10 bash -c "'$app' remote view | grep -q 'no jam'"
    "$app" login "$APP_URL|$USER|$PASS" >/dev/null; wait_for server "$APP_URL" 30 >/dev/null

    # This phone a guest of a jam on another server (the relay's), hosted on this computer (tools/jam-host.py),
    # while its own profile is the home server's.
    echo "  -- a guest of a jam on another server"
    relay_song() { curl -s "$jam_server/rest/search3?u=admin&p=admin&v=1.16.1&c=e2e&f=json&songCount=1&albumCount=0&artistCount=0&query=$1" | python3 -c "
import sys,json
s=json.load(sys.stdin)['subsonic-response']['searchResult3']['song'][0]
print(json.dumps({k:s.get(k) for k in ['id','title','artist','album','albumId','coverArt','duration']}))"; }
    rm -f "$guests/host.log"
    python3 "$here/jam-host.py" "$jam_server" admin admin "$(relay_song 'Long%20Track%2004')" "$(relay_song 'Long%20Track%2005')" > "$guests/host.log" 2>&1 & host=$!
    wait_until 15 grep -qs invite: "$guests/host.log"
    link=$(grep invite: "$guests/host.log" | cut -d' ' -f2)
    # The invite is the relay's page: the browser opens it, and the page hands the invite to the app.
    check "the guest invite reaches nori" open_invite "$link" || return 1
    check "the invite opens the player on the host's jam" wait_until 20 on_screen 'text="Mac Host’s Jam"' || return 1
    check "playing what the host plays" wait_for title "Long Track 04" 15
    check "the jam button says so" wait_until 10 on_screen 'content-desc="Jam · Mac Host · Not playing here"'
    no_controls() { off_screen 'content-desc="Next"' && off_screen 'content-desc="Shuffle"' && off_screen 'content-desc="Remove"'; }
    check "with no controls of its own" no_controls
    check "watching is clearly marked" wait_until 10 on_screen 'text="Watching the Jam · Tap Listen here to hear the music"'
    check "the host's clock advances while watching" wait_for advancing True 10
    "$app" open library >/dev/null
    guest_library() { on_screen 'text="Albums"' && on_screen 'text="Artists"' && on_screen 'text="Genres"' && off_screen 'text="Playlists"'; }
    check "its Library is the host's, without the account's playlists" wait_until 10 guest_library
    far=$(relay_song 'Far%20Song%20Two' | python3 -c 'import sys,json; print(json.load(sys.stdin)["albumId"])')
    "$app" open "album/$far" >/dev/null
    wait_until 15 on_screen 'text="Far Song Two"'
    tapnode text "Far Song Two"
    check "a tap asks the host, and the row says so" wait_until 10 on_screen 'text="Asked"'
    check "the host takes it" wait_until 20 grep -q "accepted: Far Song Two" "$guests/host.log"
    check "and the row lets go" wait_until 10 off_screen 'text="Asked"'
    "$app" open queue >/dev/null
    check "the host's queue says it was asked for here" wait_until 10 on_screen 'content-desc="Added by '
    watch_from_now
    tapnode text "Listen here"
    check "Listen here plays the host's music on this phone" sounds 30
    check "the guest follows before it starts" wait_until 10 grep -q 'following: Start' "$watching"
    check "the guest timeline never seeks back to the song's start" bash -c '! grep -Eq "rust player: to song [0-9]+ at 0 ms" "$1"' _ "$watching"
    # Listening along is playback as any other: the service in the foreground, its notification the
    # jam's song with whose jam it is, on through the background and the screen off (the deep buffer's
    # bursts keep coming).
    jam_notified() { adb shell dumpsys notification --noredact | grep -A40 'pkg=dev.nori.music' | grep -q 'android.subText=String (Jam · Mac Host)'; }
    check "the notification says whose jam it is" wait_until 10 jam_notified
    in_foreground() { adb shell dumpsys activity services dev.nori.music | grep -q 'isForeground=true'; }
    adb shell input keyevent 3
    adb shell input keyevent 26
    check "in the background, screen off, the service stays in the foreground" in_foreground
    check "and the music goes on" bursts_continue
    check "and on" bursts_continue
    "$app" wake >/dev/null
    "$app" launch >/dev/null
    check "back in the app, the jam plays here" wait_for playing True 10
    # A plain guest's pause holds its own listening (the host plays on); play joins the jam where it is then.
    "$app" do pause >/dev/null
    check "a guest's pause is its own" silent 10
    check "the player says the jam plays on" wait_until 10 on_screen 'text="Paused here · Jam still playing"'
    "$app" do resume >/dev/null
    check "play joins the jam again" sounds 15
    check "the strip says the jam again" wait_until 10 off_screen 'text="Paused here · Jam still playing"'
    tapnode content-desc Leave
    check "leaving returns to the home server" wait_for server "$APP_URL" 20
    check "and its music stops at once" silent 5
    check "the host saw it leave" wait_until 15 grep -q "left:" "$guests/host.log"
    kill $host 2>/dev/null

    # An admin's controls are the host's: its pause pauses the jam (and so here), its play and skip too.
    echo "  -- an admin of a jam on another server"
    rm -f "$guests/admin-host.log"
    NORI_JAM_ADMINS=1 python3 "$here/jam-host.py" "$jam_server" admin admin "$(relay_song 'Long%20Track%2004')" "$(relay_song 'Long%20Track%2005')" > "$guests/admin-host.log" 2>&1 & host=$!
    wait_until 15 grep -qs invite: "$guests/admin-host.log"
    link=$(grep invite: "$guests/admin-host.log" | cut -d' ' -f2)
    check "the admin invite reaches nori" open_invite "$link" || return 1
    check "the invite opens the player on the host's jam" wait_until 20 on_screen 'text="Listen here"'
    tapnode text "Listen here"
    check "Listen here plays the host's music" sounds 30
    check "an admin skips" wait_until 10 on_screen 'content-desc="Next"'
    "$app" do pause >/dev/null
    check "an admin's pause is the host's" wait_until 15 grep -q "obeyed: pause" "$guests/admin-host.log"
    check "and pauses here with it" silent 15
    "$app" do resume >/dev/null
    check "its play is the host's" wait_until 15 grep -q "obeyed: play" "$guests/admin-host.log"
    check "and plays here with it" sounds 15
    "$app" do next >/dev/null
    check "its skip is the host's" wait_until 15 grep -q "obeyed: next" "$guests/admin-host.log"
    check "and here it plays the host's next song" wait_for title "Long Track 05" 20
    "$app" open "album/$far" >/dev/null
    wait_until 15 on_screen 'text="Far Song Two"'
    tapnode text "Far Song Two"
    check "an admin's song tap directly plays on the host" wait_until 15 grep -q "obeyed: replace" "$guests/admin-host.log"
    check "the admin sees the new song" wait_for title "Far Song Two" 15
    check "the admin's tap sends no approval request" bash -c '! grep -q "asked:" "$1"' _ "$guests/admin-host.log"
    "$app" open queue >/dev/null
    "$app" do "enqueue search:Long Track 04" >/dev/null
    check "an admin's queue addition goes straight to the host" wait_until 15 grep -q "obeyed: add" "$guests/admin-host.log"
    check "the admin sees the updated queue" wait_for queue 4 15
    check "the admin sees the queue confirmation" wait_until 5 on_screen 'text="Added to queue"'
    tapnode content-desc Leave
    check "leaving returns to the home server" wait_for server "$APP_URL" 20
    kill $host 2>/dev/null

    "$app" play "$LYRICS_SONG" >/dev/null
    check "a song played after leaving plays here" sounds 20
    check "it is the one picked" wait_for title "${LYRICS_SONG#search:}" 10
    "$app" do pause >/dev/null
    kill $host 2>/dev/null

    # A guest whose jam ends goes home by itself: at once when the host ends it, and on opening the app
    # when the relay dropped it meanwhile (its key no longer signs in).
    host_jam() { # host_jam <log>: a jam hosted on this computer, joined from this phone
      rm -f "$1"
      python3 "$here/jam-host.py" "$jam_server" admin admin "$(relay_song 'Long%20Track%2004')" > "$1" 2>&1 & host=$!
      wait_until 15 grep -qs invite: "$1"
      check "the invite reaches nori" open_invite "$(grep invite: "$1" | cut -d' ' -f2)" || return 1
      check "a guest again" wait_until 20 bash -c "'$app' remote view | grep -q 'Mac Host:HOST'"
    }
    host_jam "$guests/host-ends.log" || return 1
    kill $host
    check "the host ending the jam takes the guest home" wait_for server "$APP_URL" 15
    check "and says who ended it" wait_until 5 on_screen 'text="Mac Host ended the Jam"'
    host_jam "$guests/host-gone.log" || return 1
    kill -9 $host; wait $host 2>/dev/null
    adb shell am force-stop "$pkg"
    curl -sf "$jam_server/rest/noriRemote.close?u=admin&p=admin&v=1.16.1&c=e2e&f=json&room=$(grep room: "$guests/host-gone.log" | cut -d' ' -f2)" >/dev/null
    "$app" wake >/dev/null
    "$app" launch >/dev/null
    wait_until 20 bash -c "'$app' state 2>/dev/null | grep -q '\"route\"'" || return 1
    check "a guest profile whose jam the relay dropped opens the user's own" wait_for server "$APP_URL" 20
  fi
}
if want jam; then
  failures_before=$fail
  jam_checks || { [ "$fail" -gt "$failures_before" ] || check "Jam setup succeeds" false; }
  for guest_pid in "${gus:-}" "${dee:-}" "${host:-}"; do
    [ -z "$guest_pid" ] || kill "$guest_pid" 2>/dev/null
  done
  "$app" set remoteControl false >/dev/null
  "$app" login "$APP_URL|$USER|$PASS" >/dev/null
fi

restore_settings
feature_finished=1
finish
