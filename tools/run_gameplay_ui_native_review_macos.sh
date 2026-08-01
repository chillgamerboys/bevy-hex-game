#!/bin/zsh
set -euo pipefail

if [[ "${CONDUCTOR_IS_LOCAL:-1}" != "1" || "$(uname -s)" != "Darwin" ]]; then
  print -u2 "native gameplay UI review requires a local macOS workspace"
  exit 2
fi
if ! /usr/bin/swift - <<'SWIFT'
import CoreGraphics
exit(CGPreflightScreenCaptureAccess() ? 0 : 1)
SWIFT
then
  print -u2 "native review is BLOCKED: macOS Screen & System Audio Recording permission is missing"
  print -u2 "Open System Settings -> Privacy & Security -> Screen & System Audio Recording, enable Conductor (and the terminal host if listed), then fully restart Conductor."
  exit 3
fi
permission_probe="$(mktemp -t hex-game-screen-capture).png"
if ! /usr/sbin/screencapture -x -R0,0,1,1 "$permission_probe" 2>/dev/null; then
  rm -f "$permission_probe"
  print -u2 "native review is BLOCKED: this Conductor agent cannot capture a macOS window"
  print -u2 "Open System Settings -> Privacy & Security -> Screen & System Audio Recording, enable Conductor (and the terminal host if listed), then fully restart Conductor."
  exit 3
fi
rm -f "$permission_probe"

commit_sha="$(git rev-parse HEAD)"
review_root="${1:-.context/ui-native-review/${commit_sha}}"
data_dir="${review_root}/isolated-data"
capture_dir="${review_root}/captures"
if [[ -e "$data_dir" || -e "$capture_dir" ]]; then
  print -u2 "native review refuses reused state: choose a fresh output directory"
  print -u2 "existing path: ${review_root}"
  exit 4
fi
mkdir -p "$data_dir" "$capture_dir"

run_phase() {
  local script="$1"
  HEX_GAME_DATA_DIR="$data_dir" \
  HEX_WALK_SCRIPT="$script" \
  HEX_WALK_OUT="$capture_dir" \
  HEX_WALK_SIZE=1280x720 \
  HEX_WALK_NATIVE_CAPTURE=tools/capture_macos_game_window.sh \
    cargo run -p hex_game --features visual-walk
}

run_phase walks/gameplay_ui_native.ron
run_phase walks/gameplay_ui_native_prepare_200.ron
run_phase walks/gameplay_ui_native_restart_200.ron

print "native UI review complete at ${commit_sha}"
print "captures: ${capture_dir}"
