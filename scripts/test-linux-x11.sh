#!/usr/bin/env bash
# Run inside the disposable Rocky CI container after packaging.
set -euo pipefail
image="${1:?Packaged AppImage required}"

# Verify both ordinary X11 and the older NX display used by X2Go.
Xvfb :99 -screen 0 1920x1200x24 -nolisten tcp &
display_pid=$!
app_pid=''
nx_pid=''
host_wm_pid=''
base_wm_pid=''
cleanup() {
  if test -n "$app_pid"; then
    kill "$app_pid" 2>/dev/null || true
    wait "$app_pid" 2>/dev/null || true
  fi
  if test -n "$host_wm_pid"; then
    kill "$host_wm_pid" 2>/dev/null || true
    wait "$host_wm_pid" 2>/dev/null || true
  fi
  if test -n "$nx_pid"; then
    kill "$nx_pid" 2>/dev/null || true
    wait "$nx_pid" 2>/dev/null || true
  fi
  if test -n "$base_wm_pid"; then
    kill "$base_wm_pid" 2>/dev/null || true
    wait "$base_wm_pid" 2>/dev/null || true
  fi
  kill "$display_pid" 2>/dev/null || true
  wait "$display_pid" 2>/dev/null || true
}
trap cleanup EXIT
for attempt in {1..50}; do
  test -S /tmp/.X11-unix/X99 && break
  kill -0 "$display_pid"
  sleep 0.1
done
test -S /tmp/.X11-unix/X99
probe_status=0
DISPLAY=:9876 "${CARGO_TARGET_DIR}/check-x11-capabilities" || probe_status=$?
test "$probe_status" -eq 20
DISPLAY=:99 "${CARGO_TARGET_DIR}/check-x11-capabilities"
DISPLAY=:99 LIBGL_ALWAYS_SOFTWARE=1 APPIMAGE_EXTRACT_AND_RUN=1 \
  timeout 30s "$image" &
app_pid=$!
window_opened=false
for attempt in {1..60}; do
  if DISPLAY=:99 xwininfo -root -tree | awk '/"Versus/{found=1} END {exit !found}'; then
    echo 'Packaged Versus window opened on Rocky Linux 8.'
    window_opened=true
    break
  fi
  kill -0 "$app_pid"
  sleep 0.25
done
if ! "$window_opened"; then
  echo 'Packaged GUI did not create a Versus window on Rocky Linux 8.' >&2
  exit 1
fi
# Check actual mappings as well as the final RUNPATH invariant: the ordinary
# app may load its keyboard libraries, but graphics/toolkit files stay native.
python3 - <<'PYMAPS'
from pathlib import Path
found = False
for process in Path("/proc").iterdir():
    if not process.name.isdigit():
        continue
    try:
        executable = (process / "exe").resolve(strict=True)
        if executable.name != "versus" or executable.parent.name != "bin":
            continue
        found = True
        environment = (process / "environ").read_bytes().split(b"\0")
        if b"VERSUS_X11_COMPAT=1" in environment:
            raise SystemExit("Modern X11 incorrectly selected compatibility mode")
        root = executable.parents[2]
        private = str(root / "usr/lib") + "/"
        native = str(root / "usr/lib/native") + "/"
        mappings = (process / "maps").read_text().splitlines()
        unexpected = [line.split()[-1] for line in mappings
                      if private in line and native not in line]
        if unexpected:
            raise SystemExit("Ordinary launch mapped private runtime libraries: " +
                             ", ".join(unexpected))
    except (FileNotFoundError, PermissionError):
        continue
if not found:
    raise SystemExit("Cannot inspect the ordinary packaged Versus process")
print("Ordinary launch keeps the private graphics/toolkit runtime isolated.")
PYMAPS
kill "$app_pid"
wait "$app_pid" || true
app_pid=''

# nxagent switches to fullscreen and later restores a smaller saved size when
# its parent display has no window manager. Give the test a normal parent desktop
# so its requested geometry is stable and pointer targets remain on-screen.
DISPLAY=:99 openbox --sm-disable --config-file /etc/xdg/openbox/rc.xml &
base_wm_pid=$!
for attempt in {1..100}; do
  if DISPLAY=:99 xprop -root _NET_SUPPORTING_WM_CHECK | grep -q 'window id # 0x'; then
    break
  fi
  kill -0 "$base_wm_pid"
  sleep 0.1
done
DISPLAY=:99 xprop -root _NET_SUPPORTING_WM_CHECK | grep -q 'window id # 0x'

# These displays exist only inside the disposable build container. nxagent is
# the legacy server behind standard X2Go, rather than a modern Xvfb substitute.
# nxagent writes its compiled host keymap here on Rocky 8. The minimal
# container does not create this runtime directory through a desktop session.
mkdir -p /usr/share/X11/xkb/compiled
# Leave room for the outer window decorations and the largest resize case.
# Otherwise the host pointer cannot reach the private desktop's bottom buttons.
DISPLAY=:99 nxagent :100 -geometry 1600x1000+0+0 -nolisten tcp &
nx_pid=$!
for attempt in {1..100}; do
  DISPLAY=:100 xdpyinfo >/dev/null 2>&1 && break
  kill -0 "$nx_pid"
  sleep 0.1
done
DISPLAY=:100 xdpyinfo -ext XInputExtension
DISPLAY=:100 xdpyinfo | grep -E 'dimensions:[[:space:]]+1600x1000[[:space:]]' >/dev/null
# An ordinary host window manager gives the outer display the same close
# protocol a user gets from the title-bar button in a remote desktop.
DISPLAY=:100 openbox --sm-disable --config-file /etc/xdg/openbox/rc.xml &
host_wm_pid=$!
for attempt in {1..100}; do
  if DISPLAY=:100 xprop -root _NET_SUPPORTING_WM_CHECK | grep -q 'window id # 0x'; then
    break
  fi
  kill -0 "$host_wm_pid"
  sleep 0.1
done
DISPLAY=:100 xprop -root _NET_SUPPORTING_WM_CHECK | grep -q 'window id # 0x'
# nxagent must reproduce the unsupported XI2 result that triggers fallback.
probe_status=0
DISPLAY=:100 "${CARGO_TARGET_DIR}/check-x11-capabilities" || probe_status=$?
test "$probe_status" -eq 10
DISPLAY=:100 PYTHONDONTWRITEBYTECODE=1 python3 scripts/smoke-linux-x11.py "$image"
