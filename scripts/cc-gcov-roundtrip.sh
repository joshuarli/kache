#!/bin/sh
# CCP-05 coverage round trip: compile a two-unit program with --coverage raw
# and through kache (cold, then warm from the store after deleting every
# object and notes file), link, run, and require the coverage tool to report
# the same counts for both, with no stamp mismatch between a restored object
# and its restored notes. The warm build must restore both units.
#
# usage: [GCOV=tool] cc-gcov-roundtrip.sh KACHE WORKDIR [CC]
# GCOV defaults to gcov for GCC and `llvm-cov gcov` for clang.
set -eu
kache=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
work=$2
cc=${3:-cc}
case $cc in
  clang*) gcov=${GCOV:-llvm-cov gcov} ;;
  *) gcov=${GCOV:-gcov} ;;
esac

mkdir -p "$work"
work=$(cd "$work" && pwd)
export KACHE_CACHE_DIR="$work/kache-store" KACHE_CONFIG="$work/kache-config.toml"
: > "$KACHE_CONFIG"

tree=$work/tree
# usage: run COMPILER [restore]
run() {
  compiler=$1
  restore=${2:-}
  rm -rf "$tree"
  mkdir -p "$tree"
  cat > "$tree/lib.c" <<'EOF'
int classify(int x) {
  if (x < 0)
    return -1;
  if (x == 0)
    return 0;
  return 1;
}
EOF
  cat > "$tree/main.c" <<'EOF'
int classify(int);
int main(void) {
  int sum = 0;
  for (int i = -2; i < 5; i++)
    sum += classify(i);
  return sum == 2 ? 0 : 1;
}
EOF
  (
    cd "$tree"
    # shellcheck disable=SC2086 # the compiler and tool strings are split on purpose
    for unit in lib main; do $compiler --coverage -O0 -c "$unit.c" -o "$unit.o"; done
    if [ -n "$restore" ]; then
      rm -f lib.o main.o lib.gcno main.gcno
      # shellcheck disable=SC2086
      for unit in lib main; do $compiler --coverage -O0 -c "$unit.c" -o "$unit.o"; done
    fi
    "$cc" --coverage lib.o main.o -o prog
    ./prog
    # shellcheck disable=SC2086
    $gcov lib.c main.c >/dev/null 2>gcov.err
  ) || { echo "the $compiler build, run or coverage report failed" >&2; cat "$tree/gcov.err" >&2 2>/dev/null; exit 1; }
  if grep -i -E 'stamp|mismatch|cannot open' "$tree/gcov.err" >&2; then
    echo "the coverage tool rejected the $compiler notes" >&2
    exit 1
  fi
  cat "$tree/lib.c.gcov" "$tree/main.c.gcov"
}

raw=$(run "$cc")

"$kache" daemon stop >/dev/null 2>&1 || true
rm -rf "$KACHE_CACHE_DIR"
warm=$(run "$kache $cc" restore)
"$kache" daemon stop >/dev/null 2>&1 || true

[ "$raw" = "$warm" ] || {
  echo "coverage reports differ between raw and kache-restored builds" >&2
  printf '%s\n' "$raw" > "$work/raw.gcov"
  printf '%s\n' "$warm" > "$work/kache.gcov"
  diff "$work/raw.gcov" "$work/kache.gcov" >&2 || true
  exit 1
}
results=$(grep -o '"result":"[a-z_]*"' "$KACHE_CACHE_DIR/events.jsonl" | tr -d '"' | sort | uniq -c)
echo "$results"
hits=$(echo "$results" | awk '$2 == "result:local_hit" {print $1}')
[ "${hits:-0}" -eq 2 ] || { echo "the warm build restored ${hits:-0} of 2 units" >&2; exit 1; }
echo "$(printf '%s\n' "$raw" | grep -Ec '^[[:space:]]*[0-9]+[*]?:') executed lines report identically; PASS"
