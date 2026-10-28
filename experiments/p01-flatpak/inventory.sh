#!/usr/bin/env bash
# Run inside the actual SDK sandbox, with no Nix tool paths in PATH.
set -euo pipefail
export PATH=/usr/lib/sdk/rust-stable/bin:/usr/lib/sdk/llvm22/bin:/usr/bin
export LD_LIBRARY_PATH=/usr/lib/sdk/llvm22/lib
missing=0
for tool in rustc cargo cc c++ clang llvm-config llvm-ar llvm-objdump llvm-profdata cmake ninja make python3 perl pkg-config; do
  if command -v "$tool"; then
    case "$tool" in
      rustc) rustc -Vv ;;
      cargo|cc|clang|cmake|ninja) "$tool" --version ;;
    esac
  else
    printf 'MISSING TOOL: %s\n' "$tool"
    missing=1
  fi
done
for library in fontconfig freetype2 wayland-client xkbcommon egl gl x11 xcursor xi xrandr; do
  if pkg-config --modversion "$library"; then
    printf 'FOUND LIBRARY: %s\n' "$library"
  else
    printf 'MISSING LIBRARY: %s\n' "$library"
    missing=1
  fi
done
printf '\nSDK libclang candidates:\n'
find /usr/lib -name 'libclang.so*' -print
if [[ ! -s /usr/lib/sdk/llvm22/lib/libclang.so ]]; then
  printf '%s\n' 'MISSING LIBRARY: LLVM 22 libclang'
  missing=1
fi
printf '\nSDK certificate candidates:\n'
for path in /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-bundle.crt /etc/pki/tls/certs/ca-bundle.crt; do
  if [[ -s "$path" ]]; then printf '%s\n' "$path"; fi
done
printf '\nSDK font families:\n'
fc-list --format='%{family}\n' | sort -u
exit "$missing"
