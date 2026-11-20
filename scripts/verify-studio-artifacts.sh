#!/usr/bin/env bash
set -euo pipefail
dir=${1:?usage: verify-studio-artifacts.sh DIRECTORY ARCH}
arch=${2:?usage: verify-studio-artifacts.sh DIRECTORY ARCH}
case "$arch" in amd64|arm64) ;; *) echo "unknown arch: $arch" >&2; exit 1;; esac
# One OS builds one artifact set (release-studio-tauri.yml Collect artifacts):
# darwin dmg+zip, windows installer, linux deb/rpm/AppImage. A per-OS call
# checks only what that OS's runner produced.
case "$(uname -s)" in
  Darwin)
    names=("ReasonixStudio-darwin-$arch.dmg" "ReasonixStudio-darwin-$arch.zip")
    ;;
  Linux)
    names=("ReasonixStudio-linux-$arch.deb" "ReasonixStudio-linux-$arch.rpm" "ReasonixStudio-linux-$arch.AppImage")
    ;;
  *)
    names=("ReasonixStudio-windows-$arch-installer.exe")
    ;;
esac
for name in "${names[@]}"; do
  test -s "$dir/$name" || { echo "missing or empty Studio artifact: $name" >&2; exit 1; }
done
