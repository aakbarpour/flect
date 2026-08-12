#!/usr/bin/env bash
set -euo pipefail

target="${1:?target triple is required}"
version="${2:?version is required}"
case "$target" in
  *-windows-*) binary="flect.exe" ;;
  *) binary="flect" ;;
esac

name="flect-${version}-${target}"
stage="dist/${name}"
rm -rf "$stage"
mkdir -p "$stage"
cp "target/${target}/release/${binary}" "$stage/${binary}"
cp LICENSE README.md "$stage/"
tar -C dist -czf "dist/${name}.tar.gz" "$name"
rm -rf "$stage"

