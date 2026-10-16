#!/usr/bin/env bash
# Build caprust-otio-import and install it into CapRust's importer
# directory so the File menu picks it up.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/.." && pwd)"
name=caprust-otio-import

echo "building..."
cargo build --release --manifest-path "$root/Cargo.toml" -p "$name"

# The target directory is wherever cargo puts it, which can be shared
# across projects rather than sitting next to the source tree.
target="$(cargo metadata --manifest-path "$root/Cargo.toml" --format-version 1 --no-deps \
  | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"
[ -n "$target" ] || { echo "Could not find the cargo target directory."; exit 1; }

if [ -n "${APPDATA:-}" ]; then
  dest="${APPDATA}/CapRust/importers"
else
  dest="$HOME/.config/CapRust/importers"
fi
mkdir -p "$dest"

case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*)
    cp "$target/release/$name.exe" "$dest/$name.exe"
    ;;
  *)
    cp "$target/release/$name" "$dest/$name"
    ;;
esac

cat > "$dest/$name.json" <<'JSON'
{
  "id": "org.caprust.otio",
  "name": "Import OTIO...",
  "version": "0.1.0",
  "protocol": 1,
  "kinds": ["otio"],
  "target": "file"
}
JSON

echo
echo "installed in $dest"
ls -la "$dest"
