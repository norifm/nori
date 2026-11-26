#!/usr/bin/env bash
# Make nori-test, the AVD the device checks run on: the plain AOSP image (no Play services) on a small
# screen, so a headless emulator with software graphics stays cheap. Not an ATD image: it leaves out the
# volume panel and the output switcher that the remote checks drive.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
name="${1:-nori-test}"
sdk="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$(sed -n 's/^sdk\.dir=//p' "$here/../local.properties")}}"
case "$(uname -m)" in arm64|aarch64) abi=arm64-v8a ;; *) abi=x86_64 ;; esac
image="system-images;android-36;default;$abi"
tools="$sdk/cmdline-tools/latest/bin"
# `yes` accepts the image's license; it ends on a closed pipe, so its status is not the install's.
(set +o pipefail; yes | "$tools/sdkmanager" "$image" >/dev/null 2>&1) ||
  { echo "sdkmanager could not install $image" >&2; exit 1; }
# It may complain that the image has no devices.xml; it makes the AVD anyway.
echo no | "$tools/avdmanager" create avd --force -n "$name" -k "$image" -d pixel_4 >/dev/null 2>&1 ||
  { echo "avdmanager could not make $name" >&2; exit 1; }
config="${ANDROID_AVD_HOME:-$HOME/.android/avd}/$name.avd/config.ini"
python3 - "$config" <<'PY'
import sys
path = sys.argv[1]
want = {"hw.ramSize": "2048", "hw.cpu.ncore": "2", "hw.lcd.width": "720", "hw.lcd.height": "1280",
        "hw.lcd.density": "320", "vm.heapSize": "256", "disk.dataPartition.size": "6G",
        "hw.audioInput": "no", "hw.camera.back": "none", "hw.camera.front": "none", "hw.gps": "no",
        "hw.keyboard": "yes", "showDeviceFrame": "no"}
kept = [line for line in open(path).read().splitlines() if line.split("=")[0].strip() not in want]
open(path, "w").write("\n".join(kept + [f"{k}={v}" for k, v in want.items()]) + "\n")
PY
echo "$name ready: tools/emulator.sh $name 5554"
