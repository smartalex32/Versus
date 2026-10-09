#!/usr/bin/env bash
# AppRun for the Linux AppImage.  --compat-x11 starts Versus inside a private
# nested Xephyr server for older X11 hosts, including X2Go/nxagent.
set -euo pipefail

die() {
  echo "Versus launcher: $*" >&2
  exit 1
}

script_dir="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
appdir="${APPDIR:-$script_dir}"
versus="$appdir/usr/bin/versus"

test -x "$versus" || die "missing bundled executable: $versus"
# linuxdeploy's generated AppRun normally supplies this path.  This custom
# launcher keeps the ordinary keyboard runtime separate from the private
# graphics/helper runtime, while allowing a host-provided path to remain.
export LD_LIBRARY_PATH="$appdir/usr/lib/native${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export PATH="$appdir/usr/bin:$PATH"
export XDG_DATA_DIRS="$appdir/usr/share:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
export GSETTINGS_SCHEMA_DIR="$appdir/usr/share/glib-2.0/schemas"

compat_x11=false
arguments=()
end_of_options=false
for argument in "$@"; do
  if ! "$end_of_options" && test "$argument" = --; then
    end_of_options=true
    arguments+=("$argument")
  elif ! "$end_of_options" && test "$argument" = --compat-x11; then
    compat_x11=true
  else
    arguments+=("$argument")
  fi
done

# The CLI handles these before creating a window.  Keep that property even if
# callers mechanically append --compat-x11 to all launches.
end_of_options=false
for argument in "${arguments[@]}"; do
  "$end_of_options" && break
  case "$argument" in
    --) end_of_options=true ;;
    --help|--version)
      exec "$versus" "${arguments[@]}"
      ;;
  esac
done

if ! "$compat_x11"; then
  exec "$versus" "${arguments[@]}"
fi

# Keep the private graphics/toolkit runtime out of ordinary native launches.
export LD_LIBRARY_PATH="$appdir/usr/lib:$LD_LIBRARY_PATH"

host_display="${DISPLAY:-}"
test -n "$host_display" || die "--compat-x11 requires an existing X11 DISPLAY"

xephyr="$appdir/usr/bin/Xephyr"
xdpyinfo="$appdir/usr/bin/xdpyinfo"
xauth="$appdir/usr/bin/xauth"
xkbcomp="$appdir/usr/bin/xkbcomp"
openbox="$appdir/usr/bin/openbox"
xprop="$appdir/usr/bin/xprop"
xkb_dir="$appdir/usr/share/X11/xkb"
mesa_dri="$appdir/usr/lib/dri"
openbox_config="$appdir/usr/share/versus/compat-openbox.xml"
for required in "$xephyr" "$xdpyinfo" "$xauth" "$xkbcomp" "$openbox" "$xprop"; do
  test -x "$required" || die "missing bundled compatibility helper: $required"
done
test -d "$xkb_dir" || die "missing bundled XKB data: $xkb_dir"
test -d "$mesa_dri" || die "missing bundled Mesa DRI drivers: $mesa_dri"
test -r "$openbox_config" || die "missing bundled Openbox configuration: $openbox_config"
export __EGL_VENDOR_LIBRARY_FILENAMES="$appdir/usr/share/glvnd/egl_vendor.d/50_mesa.json"

umask 077
state_dir="$(mktemp -d "${TMPDIR:-/tmp}/versus-x11.XXXXXX")" || die "cannot create private temporary directory"
display_file="$state_dir/display"
authority_file="$state_dir/Xauthority"
: > "$authority_file"
chmod 600 "$authority_file"

app_pid=''
xephyr_pid=''
openbox_pid=''

process_running() {
  local pid="$1"
  if test -r "/proc/$pid/stat"; then
    # kill -0 reports zombies as alive.  The Linux launcher needs to observe a
    # closed Xephyr promptly so it can end the nested client too.
    test "$(awk '{print $3}' "/proc/$pid/stat")" != Z
  else
    kill -0 "$pid" 2>/dev/null
  fi
}

stop_process() {
  local pid="$1"
  local attempt
  stopped_status=0
  test -n "$pid" || return 0
  if process_running "$pid"; then
    kill -TERM "$pid" 2>/dev/null || true
    for attempt in 1 2 3 4 5 6 7 8 9 10; do
      process_running "$pid" || break
      sleep 0.1
    done
    process_running "$pid" && kill -KILL "$pid" 2>/dev/null || true
  fi
  wait "$pid" 2>/dev/null || stopped_status=$?
}

cleanup() {
  set +e
  trap - EXIT HUP INT TERM
  stop_process "$app_pid"
  stop_process "$openbox_pid"
  stop_process "$xephyr_pid"
  rm -rf -- "$state_dir"
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

cookie="$(od -An -N16 -tx1 /dev/urandom | tr -d '[:space:]')"
test "${#cookie}" -eq 32 || die "could not generate an X11 authorization cookie"
# The server reads the authority file while it starts, before -displayfd tells
# us its chosen number.  FamilyWild (ffff) deliberately matches every display,
# allowing a private MIT cookie to be present from startup without guessing a
# free :N.  Add the concrete mapping after allocation for ordinary clients.
"$xauth" -f "$authority_file" add :0 MIT-MAGIC-COOKIE-1 "$cookie"
"$xauth" -f "$authority_file" nlist :0 | sed 's/^..../ffff/' | "$xauth" -f "$authority_file" nmerge -


# Xephyr's -displayfd asks the server to allocate its own display number.  It
# eliminates the racy "find a free :N" probe while the FamilyWild record above
# keeps authentication enabled before the number is known.
exec {display_fd}>"$display_file"
if test -v XAUTHORITY; then
  DISPLAY="$host_display" XAUTHORITY="$XAUTHORITY" \
    PATH="$appdir/usr/bin:$PATH" XKB_CONFIG_ROOT="$xkb_dir" \
    LIBGL_ALWAYS_SOFTWARE=1 LIBGL_DRIVERS_PATH="$mesa_dri" \
    "$xephyr" -displayfd "$display_fd" -auth "$authority_file" -nolisten tcp \
      -noreset -resizeable -screen 1280x800 -xkbdir "$xkb_dir" &
else
  DISPLAY="$host_display" PATH="$appdir/usr/bin:$PATH" XKB_CONFIG_ROOT="$xkb_dir" \
    LIBGL_ALWAYS_SOFTWARE=1 LIBGL_DRIVERS_PATH="$mesa_dri" \
    "$xephyr" -displayfd "$display_fd" -auth "$authority_file" -nolisten tcp \
      -noreset -resizeable -screen 1280x800 -xkbdir "$xkb_dir" &
fi
xephyr_pid=$!
eval "exec ${display_fd}>&-"

readiness_timeout="${VERSUS_X11_READINESS_TIMEOUT:-10}"
case "$readiness_timeout" in
  ''|*[!0-9]*) die "VERSUS_X11_READINESS_TIMEOUT must be a whole number" ;;
esac
deadline=$((SECONDS + readiness_timeout))
display_number=''
while test "$SECONDS" -le "$deadline"; do
  if test -s "$display_file"; then
    display_number="$(tr -d '\r\n' < "$display_file")"
    case "$display_number" in
      ''|*[!0-9]*) die "Xephyr returned an invalid display number" ;;
      *) break ;;
    esac
  fi
  process_running "$xephyr_pid" || die "Xephyr exited before allocating a display"
  sleep 0.05
done
test -n "$display_number" || die "Xephyr did not allocate a display within ${readiness_timeout}s"
nested_display=":$display_number"

"$xauth" -f "$authority_file" add "$nested_display" MIT-MAGIC-COOKIE-1 "$cookie"

while test "$SECONDS" -le "$deadline"; do
  if XAUTHORITY="$authority_file" "$xdpyinfo" -display "$nested_display" >/dev/null 2>&1; then
    break
  fi
  process_running "$xephyr_pid" || die "Xephyr exited before becoming ready"
  sleep 0.05
done
XAUTHORITY="$authority_file" "$xdpyinfo" -display "$nested_display" >/dev/null 2>&1 \
  || die "Xephyr did not become ready within ${readiness_timeout}s"

DISPLAY="$nested_display" XAUTHORITY="$authority_file" \
  XDG_DATA_DIRS="$appdir/usr/share" \
  "$openbox" --sm-disable --config-file "$openbox_config" &
openbox_pid=$!

while test "$SECONDS" -le "$deadline"; do
  wm_check="$(XAUTHORITY="$authority_file" "$xprop" -display "$nested_display" -root _NET_SUPPORTING_WM_CHECK 2>/dev/null || true)"
  case "$wm_check" in
    *"window id # 0x"*) break ;;
    *) ;;
  esac
  process_running "$openbox_pid" || die "Openbox exited before becoming ready"
  sleep 0.05
done
case "${wm_check:-}" in
  *"window id # 0x"*) ;;
  *) die "Openbox did not become ready within ${readiness_timeout}s" ;;
esac

DISPLAY="$nested_display" XAUTHORITY="$authority_file" VERSUS_X11_COMPAT=1 \
  LIBGL_ALWAYS_SOFTWARE=1 LIBGL_DRIVERS_PATH="$mesa_dri" \
  PATH="$appdir/usr/bin:$PATH" XKB_CONFIG_ROOT="$xkb_dir" \
  "$versus" "${arguments[@]}" &
app_pid=$!

while :; do
  if ! process_running "$app_pid"; then
    set +e
    wait "$app_pid"
    app_status=$?
    set -e
    app_pid=''
    stop_process "$openbox_pid"
    openbox_pid=''
    exit "$app_status"
  fi
  if ! process_running "$openbox_pid"; then
    # A maximized compatibility window depends on Openbox for its workspace.
    # Do not leave it running after the private window manager exits.
    stop_process "$app_pid"
    app_status=$stopped_status
    app_pid=''
    wait "$openbox_pid" 2>/dev/null || true
    openbox_pid=''
    exit "$app_status"
  fi
  if ! process_running "$xephyr_pid"; then
    # Closing the nested Xephyr window must also end the application.  Return
    # the application's resulting status so callers retain normal CLI status.
    stop_process "$app_pid"
    app_status=$stopped_status
    app_pid=''
    stop_process "$openbox_pid"
    openbox_pid=''
    wait "$xephyr_pid" 2>/dev/null || true
    xephyr_pid=''
    exit "$app_status"
  fi
  sleep 0.05
done
