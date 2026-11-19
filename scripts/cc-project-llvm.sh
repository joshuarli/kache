#!/bin/sh
# CCP-07 real-project check: build a fixed LLVM target subset pinned in
# campaigns/cpp/lock.toml (llvm-tblgen and LLVMCore, which includes headers
# TableGen generates into the build tree) with CMake and Ninja, raw and
# through kache as the compiler launcher (cold, then warm after deleting the
# build tree), in the transparent profile. Requires byte-identical objects,
# identical TableGen output from the raw and warm tools, and a warm build that
# compiles nothing.
#
# usage: cc-project-llvm.sh KACHE WORKDIR [CC]
set -eu
kache=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
work=$2
cc=${3:-cc}
case $cc in
  gcc*) cxx=g++${cc#gcc} ;;
  clang*) cxx=clang++${cc#clang} ;;
  *cc) cxx=${cc%cc}c++ ;;
  *) cxx=$cc++ ;;
esac
version=21.1.8
url=https://github.com/llvm/llvm-project/releases/download/llvmorg-$version/llvm-project-$version.src.tar.xz
sha=$(awk -F'"' '/^\[projects.llvm\]/{p=1} p && /^sha256/{print $2; exit}' "$(dirname "$0")/../campaigns/cpp/lock.toml")
targets="llvm-tblgen LLVMCore"
command -v python3 >/dev/null 2>&1 || { echo "LLVM's CMake configure needs python3" >&2; exit 1; }

mkdir -p "$work"
work=$(cd "$work" && pwd)
archive=$work/llvm-project-$version.src.tar.xz
if [ ! -f "$archive" ]; then
  if command -v curl >/dev/null 2>&1; then curl -fsSLo "$archive" "$url"; else wget -q -O "$archive" "$url"; fi
fi
got=$( (sha256sum "$archive" 2>/dev/null || shasum -a 256 "$archive") | cut -d' ' -f1)
[ "$got" = "$sha" ] || { echo "llvm-project digest $got does not match the lock ($sha)" >&2; exit 1; }

src=$work/llvm-project-$version.src
if [ ! -d "$src/llvm" ]; then
  tar xJf "$archive" -C "$work" "llvm-project-$version.src/llvm" "llvm-project-$version.src/cmake" "llvm-project-$version.src/third-party"
fi

export KACHE_CACHE_DIR="$work/kache-store" KACHE_CONFIG="$work/kache-config.toml"
export KACHE_CC_PATH_NORMALIZE=0 KACHE_CC_SOURCE_DATE_EPOCH=passthrough
: > "$KACHE_CONFIG"

# Every arm builds at the same absolute path, so paths in the outputs agree.
build=$src/build
configure() {
  rm -rf "$build"
  cmake -S "$src/llvm" -B "$build" -G Ninja \
    -DCMAKE_C_COMPILER="$cc" -DCMAKE_CXX_COMPILER="$cxx" \
    -DCMAKE_BUILD_TYPE=Release \
    -DLLVM_TARGETS_TO_BUILD=X86 \
    -DLLVM_ENABLE_ASSERTIONS=OFF \
    -DLLVM_APPEND_VC_REV=OFF \
    -DLLVM_INCLUDE_TESTS=OFF \
    -DLLVM_INCLUDE_BENCHMARKS=OFF \
    -DLLVM_INCLUDE_EXAMPLES=OFF \
    -DLLVM_ENABLE_ZLIB=OFF -DLLVM_ENABLE_ZSTD=OFF -DLLVM_ENABLE_LIBXML2=OFF \
    "$@" >"$work/configure.log"
  # shellcheck disable=SC2086 # the target list is split on purpose
  cmake --build "$build" --target $targets >"$work/build.log"
}
digests() {
  (cd "$build" && find . -name '*.o' | LC_ALL=C sort | while read -r f; do
    printf '%s ' "$f"; (sha256sum "$f" 2>/dev/null || shasum -a 256 "$f") | cut -d' ' -f1
  done)
}
tablegen() {
  "$build/bin/llvm-tblgen" -gen-intrinsic-enums -I "$src/llvm/include" \
    "$src/llvm/include/llvm/IR/Intrinsics.td" | (sha256sum 2>/dev/null || shasum -a 256) | cut -d' ' -f1
}
# Results for the build's own units. CMake's try-compile probes run in fresh
# scratch directories each configure, so they cannot be reused and are left out.
events() {
  grep -v 'CMakeScratch' "$KACHE_CACHE_DIR/events.jsonl" \
    | grep -o '"result":"[a-z_]*"' | tr -d '"' | sort | uniq -c
}

configure
digests > "$work/raw.txt"
raw_tablegen=$(tablegen)

"$kache" daemon stop >/dev/null 2>&1 || true
rm -rf "$KACHE_CACHE_DIR"
launcher="-DCMAKE_C_COMPILER_LAUNCHER=$kache -DCMAKE_CXX_COMPILER_LAUNCHER=$kache"
# shellcheck disable=SC2086
configure $launcher
digests > "$work/cold.txt"
"$kache" daemon stop >/dev/null 2>&1 || true
cold=$(events)

# shellcheck disable=SC2086
configure $launcher
digests > "$work/warm.txt"
"$kache" daemon stop >/dev/null 2>&1 || true
all=$(events)
[ "$(tablegen)" = "$raw_tablegen" ] || { echo "the warm kache llvm-tblgen writes different output" >&2; exit 1; }

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
