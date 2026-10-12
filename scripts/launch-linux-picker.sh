#!/usr/bin/env bash
# Run the bundled Zenity fallback with the complete private GTK/Mesa runtime.
set -euo pipefail

script_dir="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
appdir="${APPDIR:-$(CDPATH= cd -- "$script_dir/../.." && pwd)}"
library_dir="$appdir/usr/lib"

export LD_LIBRARY_PATH="$library_dir${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export LIBGL_ALWAYS_SOFTWARE=1
export LIBGL_DRIVERS_PATH="$library_dir/dri"
export __EGL_VENDOR_LIBRARY_FILENAMES="$appdir/usr/share/glvnd/egl_vendor.d/50_mesa.json"
export GSETTINGS_SCHEMA_DIR="$appdir/usr/share/glib-2.0/schemas"
export XDG_DATA_DIRS="$appdir/usr/share${XDG_DATA_DIRS:+:$XDG_DATA_DIRS}"

exec "$script_dir/zenity-real" "$@"
