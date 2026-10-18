#!/bin/sh
set -eu
# Per-user installation; no root privileges required.
source_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
if [ "$#" -eq 2 ] && [ "$1" = "--prefix" ]; then
  install_prefix=$2
  case "$install_prefix" in /*) ;; *) printf 'Prefix must be absolute.\n' >&2; exit 1;; esac
  data_dir="$install_prefix/share"
  bin_dir="$install_prefix/bin"
elif [ "$#" -eq 0 ]; then
  data_dir="${XDG_DATA_HOME:-$HOME/.local/share}"
  bin_dir="$HOME/.local/bin"
else
  printf 'Usage: install.sh [--prefix /absolute/path]\n' >&2
  exit 1
fi
app_dir="$data_dir/opendownload-app"
mkdir -p "$app_dir" "$bin_dir" "$data_dir/applications" "$data_dir/icons/hicolor/scalable/apps"
cp "$source_dir/opendownload" "$app_dir/opendownload"
chmod +x "$app_dir/opendownload"
cp -R "$source_dir/extensions" "$app_dir/"
ln -sf "$app_dir/opendownload" "$bin_dir/opendownload"
cp "$source_dir/opendownload.svg" "$data_dir/icons/hicolor/scalable/apps/opendownload.svg"
# Absolute Exec works even when ~/.local/bin is absent from the desktop session PATH.
python3 - "$app_dir/opendownload" "$source_dir/opendownload.desktop" "$data_dir/applications/opendownload.desktop" <<'PY'
import sys
from pathlib import Path
binary, template, output = sys.argv[1:]
escaped = binary.replace('\\', '\\\\').replace('"', '\\"').replace('`', '\\`').replace('$', '\\$').replace('%', '%%')
Path(output).write_text(Path(template).read_text().replace('Exec=opendownload', 'Exec="' + escaped + '"'))
PY
printf 'Installed. Open OpenDownload from your application menu.\n'
