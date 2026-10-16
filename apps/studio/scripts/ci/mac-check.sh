#!/bin/sh
# Runs the Mac App Store variant as the store runs it: in the App Sandbox, with the store's
# sandbox entitlements. Starts the synthetic sphere through the app's own form, photographs
# the app's own window (by window id, never the screen), and fails when the run does not
# complete or the window is blank.
#
#   scripts/ci/mac-check.sh <debug binary of the store variant> <sphere inputs folder> <out dir>
#
# A store-signed app cannot be launched before Apple delivers it (its application-identifier
# entitlement asks for a store receipt), so this signs a copy ad hoc with the same sandbox
# entitlements minus the two identifiers that only a provisioning profile can back. That is
# the only difference from what ships. Removes the app's container afterwards.
set -eu
BINARY=$1
SPHERE=$2
OUT=$3
HERE=$(cd "$(dirname "$0")" && pwd)
STUDIO=$(cd "$HERE/../.." && pwd)
BUNDLE=com.crispstrobe.crisp3ds
mkdir -p "$OUT"
WORK=$(mktemp -d)
APP="$WORK/CrispMesh.app"
mkdir -p "$APP/Contents/MacOS"
cp "$BINARY" "$APP/Contents/MacOS/crispmesh-studio"
cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>$BUNDLE</string>
<key>CFBundleExecutable</key><string>crispmesh-studio</string>
<key>CFBundleName</key><string>CrispMesh</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleVersion</key><string>1</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
EOF
# The app's own Info.plist keys, as the Tauri bundler merges them (camera and local-network usage texts): WebKit
# offers the camera API only to an app that declares camera use.
"${PYTHON:-python3}" - "$STUDIO/src-tauri/Info.plist" "$APP/Contents/Info.plist" <<'PY'
import plistlib, sys
own = plistlib.load(open(sys.argv[1], "rb"))
target = plistlib.load(open(sys.argv[2], "rb"))
target.update(own)
plistlib.dump(target, open(sys.argv[2], "wb"))
PY
"${PYTHON:-python3}" - "$STUDIO/src-tauri/entitlements.appstore.plist" "$WORK/entitlements.plist" <<'PY'
import plistlib, sys
entitlements = plistlib.load(open(sys.argv[1], "rb"))
for key in ("com.apple.application-identifier", "com.apple.developer.team-identifier"):
    entitlements.pop(key, None)
plistlib.dump(entitlements, open(sys.argv[2], "wb"))
PY
codesign --force --sign - --entitlements "$WORK/entitlements.plist" "$APP" 2>&1 | tail -1
codesign -d --entitlements - --xml "$APP" 2>/dev/null | plutil -p - | sed 's/^/   /'

CONTAINER="$HOME/Library/Containers/$BUNDLE"
cleanup() {
  [ -n "${PID:-}" ] && kill "$PID" 2>/dev/null || true
  rm -rf "$WORK" "$CONTAINER"
}
trap cleanup EXIT
rm -rf "$CONTAINER"
# The first start makes the container; the sphere goes into its data folder.
"$APP/Contents/MacOS/crispmesh-studio" >/dev/null 2>&1 &
PID=$!
n=0; until [ -d "$CONTAINER/Data/Library" ] || [ $n -ge 60 ]; do sleep 1; n=$((n+1)); done
sleep 3; kill "$PID" 2>/dev/null || true; wait "$PID" 2>/dev/null || true; PID=""
SUPPORT="$CONTAINER/Data/Library/Application Support/$BUNDLE"
mkdir -p "$SUPPORT/data"
# An inputs folder (cameras.json), or a photos object: rgb/ and a lens calibration next to it.
if [ -f "$SPHERE/cameras.json" ]; then
  cp -R "$SPHERE" "$SUPPORT/data/sphere"
  options='name: "sphere"'
else
  cp -R "$SPHERE/rgb" "$SUPPORT/data/rgb"
  mkdir -p "$SUPPORT/data/calibrations"
  cp "$SPHERE"/*.json "$SUPPORT/data/calibrations/"
  lens=$(basename "$(ls "$SPHERE"/*.json | head -n 1)" .json)
  options="name: \"$(basename "$SPHERE")\", photos: \"rgb\", calibration: \"$lens\", providers: { masks: \"threshold\", cameras: \"turntable\" }, settings: null"
fi
log="$SUPPORT/autopilot.log"
rm -f "$log"
{
  echo "window.AUTOPILOT = { $options, up: \"${STUDIO_UP:-+Y}\", marks: true, hold: 4000, linger: 3000, timeout: 3000000 };"
  cat "$STUDIO/scripts/autopilot-run.js"
} > "$CONTAINER/Data/autopilot.js"

cat > "$WORK/winid.swift" <<'SWIFT'
import CoreGraphics
import Foundation
let wanted = Int(CommandLine.arguments[1])!
guard let list = CGWindowListCopyWindowInfo([.optionAll], kCGNullWindowID) as? [[String: Any]] else { exit(1) }
// The app's largest ordinary window: the main window, not a title strip or a menu.
var best = (id: -1, area: 0.0)
for w in list where (w[kCGWindowOwnerPID as String] as? Int) == wanted && (w[kCGWindowLayer as String] as? Int ?? 1) == 0 {
    let b = w[kCGWindowBounds as String] as? [String: Any] ?? [:]
    let area = (b["Width"] as? Double ?? 0) * (b["Height"] as? Double ?? 0)
    if area > best.area { best = (w[kCGWindowNumber as String] as? Int ?? -1, area) }
}
if best.id >= 0 && best.area > 200 * 200 { print(best.id) }
SWIFT
swiftc -O -o "$WORK/winid" "$WORK/winid.swift"

CRISPMESH_STUDIO_AUTOPILOT="$CONTAINER/Data/autopilot.js" "$APP/Contents/MacOS/crispmesh-studio" >"$WORK/stdout.txt" 2>&1 &
PID=$!
shots=""
seen=""
deadline=$(( $(date +%s) + 3600 ))
while :; do
  if ! kill -0 "$PID" 2>/dev/null && ! grep -q "AUTOPILOT DONE\|AUTOPILOT FAILED" "$log" 2>/dev/null; then
    echo "the app exited on its own:"; tail -20 "$WORK/stdout.txt"; exit 1
  fi
  if [ -f "$log" ]; then
    for name in $(sed -n 's/^\[autopilot\] MARK //p' "$log"); do
      case " $seen " in *" $name "*) continue ;; esac
      seen="$seen $name"
      sleep 1
      wid=$("$WORK/winid" "$PID" || true)
      if [ -n "$wid" ]; then
        screencapture -x -o -l"$wid" "$OUT/mac-$name.png" && shots="$shots $OUT/mac-$name.png" && echo "   screenshot $name (window $wid)"
      else
        echo "   no window found for $name"
      fi
      # Tells the app it may move on to the next screen.
      touch "$(dirname "$log")/ack-$name"
    done
    if grep -q "AUTOPILOT DONE\|AUTOPILOT FAILED" "$log"; then break; fi
  fi
  if [ "$(date +%s)" -gt "$deadline" ]; then echo "timed out"; break; fi
  sleep 1
done
echo "== what the app reported"
grep -v "^\[autopilot\] *$" "$log" 2>/dev/null | sed 's/^/   /' || true
grep -q "AUTOPILOT DONE ok" "$log" || { echo "the run did not complete cleanly"; exit 1; }
test -n "$shots" || { echo "no screenshots were taken"; exit 1; }
for name in run surface sheets runs new-run; do
  case " $seen " in *" $name "*) ;; *) echo "the $name screen was not photographed"; exit 1 ;; esac
done
# shellcheck disable=SC2086
"${PYTHON:-python3}" "$HERE/image-check.py" $shots
