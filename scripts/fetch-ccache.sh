#!/bin/sh
# Fetch and verify the pinned ccache release binary for this host's
# campaign lane (campaigns/cpp/lock.toml) into DIR; print its path.
set -eu
dir=${1:?usage: fetch-ccache.sh DIR ORACLE_BINARY}
oracle=${2:?usage: fetch-ccache.sh DIR ORACLE_BINARY}
set -- $("$oracle" --print-ccache-pin)
lane=$1 url=$2 sha=$3
mkdir -p "$dir"
archive="$dir/$(basename "$url")"
if [ -x "$dir/ccache" ] && [ -f "$archive" ] && [ "$(cat "$dir/ccache.sha256" 2>/dev/null)" = "$sha" ]; then
  echo "$dir/ccache"
  exit 0
fi
if command -v curl >/dev/null 2>&1; then
  curl -fsSL --retry 2 -o "$archive.part" "$url"
else
  wget -q -O "$archive.part" "$url"
fi
got=$(sha256sum "$archive.part" | cut -d' ' -f1)
if [ "$got" != "$sha" ]; then
  echo "fetch-ccache: $url digest $got does not match the $lane pin $sha" >&2
  rm -f "$archive.part"
  exit 1
fi
mv "$archive.part" "$archive"
tar -xzf "$archive" -C "$dir"
cp "$(find "$dir" -mindepth 2 -maxdepth 2 -type f -name ccache | head -n 1)" "$dir/ccache"
chmod 755 "$dir/ccache"
echo "$sha" > "$dir/ccache.sha256"
echo "$dir/ccache"
