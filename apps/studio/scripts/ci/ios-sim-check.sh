#!/bin/sh
# Runs the iOS app in a simulator: installs the (debug) simulator build, puts the synthetic
# sphere into its Documents, starts a run through the app's own New run form, and
# photographs the screens a person sees (`xcrun simctl io screenshot`: the device's screen,
# nothing of the Mac's). Fails when the run does not complete or a screenshot is blank.
#
#   scripts/ci/ios-sim-check.sh <app> <device name> <sphere inputs folder> <out dir>
#
# The app must be a debug build (it runs scripts/autopilot-run.js only then). The simulator
# is shut down at the end. Screenshots: <out>/<device>-<screen>.png, at the device's
# native resolution (iPhone 17 Pro Max 1320x2868 and iPad Pro 13" 2064x2752 are the
# sizes App Store Connect asks for).
set -eu
APP=$1
DEVICE=$2
SPHERE=$3
OUT=$4
HERE=$(cd "$(dirname "$0")" && pwd)
BUNDLE=com.crispstrobe.crisp3ds
mkdir -p "$OUT"
tag=$(printf '%s' "$DEVICE" | tr -c 'A-Za-z0-9' '-' | sed 's/--*/-/g; s/-*$//')

xcrun simctl list devices available -j > "$OUT/.devices.json"
udid=$("${PYTHON:-python3}" -c "
import json, sys
name = sys.argv[1]
for runtime, devices in json.load(open(sys.argv[2]))['devices'].items():
    if 'iOS' in runtime:
        for d in devices:
            # An exact name, or else the first device whose name contains it (Pro Max).
            if d['name'] == name:
                print(d['udid']); sys.exit()
for runtime, devices in sorted(json.load(open(sys.argv[2]))['devices'].items(), reverse=True):
    if 'iOS' in runtime:
        for d in devices:
            if name in d['name']:
                print(d['udid']); sys.exit()
" "$DEVICE" "$OUT/.devices.json")
rm -f "$OUT/.devices.json"
test -n "$udid" || { echo "no available simulator named $DEVICE"; xcrun simctl list devices available | head -30; exit 1; }
echo "== $DEVICE ($udid)"
cleanup() { xcrun simctl shutdown "$udid" >/dev/null 2>&1 || true; }
trap cleanup EXIT
xcrun simctl boot "$udid" 2>/dev/null || true
xcrun simctl bootstatus "$udid" -b >/dev/null
# A clean status bar: the same time and full bars in every picture.
xcrun simctl status_bar "$udid" override --time 9:41 --batteryState charged --batteryLevel 100 --wifiBars 3 --cellularBars 4 >/dev/null 2>&1 || true
xcrun simctl uninstall "$udid" "$BUNDLE" >/dev/null 2>&1 || true
xcrun simctl install "$udid" "$APP"
data=$(xcrun simctl get_app_container "$udid" "$BUNDLE" data)
mkdir -p "$data/Documents/data"
# An inputs folder (cameras.json), or a photos object: rgb/ and a lens calibration next to it.
if [ -f "$SPHERE/cameras.json" ]; then
  cp -R "$SPHERE" "$data/Documents/data/sphere"
  options='name: "sphere"'
else
  cp -R "$SPHERE/rgb" "$data/Documents/data/rgb"
  mkdir -p "$data/Documents/data/calibrations"
  cp "$SPHERE"/*.json "$data/Documents/data/calibrations/"
  lens=$(basename "$(ls "$SPHERE"/*.json | head -n 1)" .json)
  options="name: \"$(basename "$SPHERE")\", photos: \"rgb\", calibration: \"$lens\", providers: { masks: \"threshold\", cameras: \"turntable\" }, settings: null"
fi
log="$data/Library/Application Support/$BUNDLE/autopilot.log"
rm -f "$log"

script="$OUT/.autopilot-$tag.js"
{
  echo "window.AUTOPILOT = { $options, up: \"${STUDIO_UP:-+Y}\", marks: true, hold: 4000, linger: 3000, timeout: 3000000 };"
  cat "$HERE/../autopilot-run.js"
} > "$script"
SIMCTL_CHILD_CRISPMESH_STUDIO_AUTOPILOT="$script" xcrun simctl launch --terminate-running-process "$udid" "$BUNDLE" >/dev/null

shots=""
seen=""
deadline=$(( $(date +%s) + 3600 ))
while :; do
  if [ -f "$log" ]; then
    for name in $(sed -n 's/^\[autopilot\] MARK //p' "$log"); do
      case " $seen " in *" $name "*) continue ;; esac
      seen="$seen $name"
      file="$OUT/$tag-$name.png"
      sleep 1
      xcrun simctl io "$udid" screenshot "$file" >/dev/null 2>&1
      shots="$shots $file"
      echo "   screenshot $name"
      # Tells the app it may move on to the next screen.
      touch "$(dirname "$log")/ack-$name"
    done
    if grep -q "AUTOPILOT DONE\|AUTOPILOT FAILED" "$log"; then break; fi
  fi
  if [ "$(date +%s)" -gt "$deadline" ]; then echo "timed out"; break; fi
  sleep 1
done
rm -f "$script"
echo "== what the app reported"
grep -v "^\[autopilot\] *$" "$log" 2>/dev/null | sed 's/^/   /' || true
grep -q "AUTOPILOT DONE ok" "$log" || { echo "the run did not complete cleanly"; exit 1; }
test -n "$shots" || { echo "no screenshots were taken"; exit 1; }
for name in run surface sheets runs new-run; do
  case " $seen " in *" $name "*) ;; *) echo "the $name screen was not photographed"; exit 1 ;; esac
done
# shellcheck disable=SC2086
"${PYTHON:-python3}" "$HERE/image-check.py" $shots
