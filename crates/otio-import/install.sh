#!/usr/bin/env bash
# Build caprust-otio-import and install it into CapRust's importer
# directory so the File menu picks it up.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/.." && pwd)"

echo "building..."
cargo build --release --manifest-path "$root/Cargo.toml" -p caprust-otio-import

if [ -n "${APPDATA:-}" ]; then
  dest="${APPDATA}/CapRust/importers"
else
  dest="$HOME/.config/CapRust/importers"
fi
mkdir -p "$dest"

case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*)
    cp "$root/target/release/caprust-otio-import.exe" "$dest/caprust-otio-import.exe"
    ;;
  *)
    cp "$root/target/release/caprust-otio-import" "$dest/caprust-otio-import"
    ;;
esac

cat > "$dest/caprust-otio-import.json" <<'JSON'
{
  "id": "org.caprust.otio",
  "name": "OTIO (.otio)...",
  "version": "0.1.0",
  "protocol": 1,
  "kinds": ["otio"]
}
JSON

echo
echo "installed in $dest"
ls -la "$dest"
