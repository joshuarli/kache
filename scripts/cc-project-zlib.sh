#!/bin/sh
# CCP-07 real-project check: build the zlib pinned in campaigns/cpp/lock.toml
# with the raw compiler and through kache (cold, then warm after a clean) in
# the transparent profile, require byte-identical objects, a
# warm build that compiles nothing, and a passing `make test`.
#
# usage: cc-project-zlib.sh KACHE WORKDIR [CC]
set -eu
kache=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
work=$2
cc=${3:-cc}
url=https://github.com/madler/zlib/releases/download/v1.3.2/zlib-1.3.2.tar.gz
sha=bb329a0a2cd0274d05519d61c667c062e06990d72e125ee2dfa8de64f0119d16

mkdir -p "$work"
work=$(cd "$work" && pwd)
archive=$work/zlib-1.3.2.tar.gz
if [ ! -f "$archive" ]; then
  if command -v curl >/dev/null 2>&1; then curl -fsSLo "$archive" "$url"; else wget -q -O "$archive" "$url"; fi
fi
got=$( (sha256sum "$archive" 2>/dev/null || shasum -a 256 "$archive") | cut -d' ' -f1)
[ "$got" = "$sha" ] || { echo "zlib digest $got does not match the lock" >&2; exit 1; }

export KACHE_CACHE_DIR="$work/kache-store" KACHE_CONFIG="$work/kache-config.toml"
export KACHE_CC_PATH_NORMALIZE=0 KACHE_CC_SOURCE_DATE_EPOCH=passthrough
: > "$KACHE_CONFIG"

# Every arm builds at the same absolute path, so paths in the outputs agree.
tree=$work/zlib-1.3.2
build() {
  rm -rf "$tree"
  tar xzf "$archive" -C "$work"
  (cd "$tree" && CC="$1" ./configure --static >/dev/null && make -s libz.a example minigzip >/dev/null)
}
digests() {
  (cd "$tree" && for f in *.o; do printf '%s ' "$f"; (sha256sum "$f" 2>/dev/null || shasum -a 256 "$f") | cut -d' ' -f1; done)
}
# Results for the build's own units, one event per line of the event log.
# configure's ztest<pid>.c probes get a new name every run, so they cannot be
# reused and are left out.
events() {
  grep -v '"crate_name":"ztest' "$KACHE_CACHE_DIR/events.jsonl" \
    | grep -o '"result":"[a-z_]*"' | tr -d '"' | sort | uniq -c
}

build "$cc"
digests > "$work/raw.txt"
(cd "$tree" && make -s test >/dev/null) || { echo "raw zlib tests fail" >&2; exit 1; }

"$kache" daemon stop >/dev/null 2>&1 || true
rm -rf "$KACHE_CACHE_DIR"
build "$kache $cc"
digests > "$work/cold.txt"
"$kache" daemon stop >/dev/null 2>&1 || true
cold=$(events)

build "$kache $cc"
digests > "$work/warm.txt"
"$kache" daemon stop >/dev/null 2>&1 || true
all=$(events)
(cd "$tree" && make -s test >/dev/null) || { echo "zlib tests fail on the warm kache build" >&2; exit 1; }

diff "$work/raw.txt" "$work/cold.txt" || { echo "cold kache objects differ from raw" >&2; exit 1; }
diff "$work/raw.txt" "$work/warm.txt" || { echo "warm kache objects differ from raw" >&2; exit 1; }
echo "objects: $(wc -l < "$work/raw.txt") identical across raw, cold kache and warm kache"
echo "cold events:"; echo "$cold"
echo "cold+warm events:"; echo "$all"
count() { echo "$1" | awk -v r="result:$2" '$2 == r {print $1}'; }
misses_cold=$(count "$cold" miss)
misses_all=$(count "$all" miss)
hits_all=$(count "$all" local_hit)
[ "${misses_cold:-0}" -gt 0 ] || { echo "the cold build recorded no kache misses; nothing went through kache" >&2; exit 1; }
[ "${misses_all:-0}" = "$misses_cold" ] || { echo "the warm build compiled again" >&2; exit 1; }
[ -z "$(count "$all" dup)" ] || { echo "the warm build compiled again (dup)" >&2; exit 1; }
[ "${hits_all:-0}" -ge "$misses_cold" ] || { echo "the warm build restored fewer units than the cold build stored" >&2; exit 1; }
echo "PASS"
