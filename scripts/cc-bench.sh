#!/bin/sh
# CCP-09 benchmark entry point: fetch and verify the pinned zlib and LLVM
# archives (untimed), then run kache-cc-bench for each toolchain.
#
# usage: cc-bench.sh BENCH KACHE CCACHE OUT TOOLCHAINS [kache-cc-bench args...]
set -eu
bench=$1 kache=$2 ccache=$3 out=$4 toolchains=$5
shift 5
inputs=$out/inputs
mkdir -p "$inputs"
lock=$(dirname "$0")/../campaigns/cpp/lock.toml
fetch() { # url file sha
  if [ ! -f "$2" ]; then
    if command -v curl >/dev/null 2>&1; then curl -fsSLo "$2" "$1"; else wget -q -O "$2" "$1"; fi
  fi
  got=$( (sha256sum "$2" 2>/dev/null || shasum -a 256 "$2") | cut -d' ' -f1)
  [ "$got" = "$3" ] || { echo "$2: digest $got does not match the lock ($3)" >&2; exit 1; }
}
fetch https://github.com/madler/zlib/releases/download/v1.3.2/zlib-1.3.2.tar.gz "$inputs/zlib-1.3.2.tar.gz" \
  bb329a0a2cd0274d05519d61c667c062e06990d72e125ee2dfa8de64f0119d16
llvm_sha=$(awk -F'"' '/^\[projects.llvm\]/{p=1} p && /^sha256/{print $2; exit}' "$lock")
fetch https://github.com/llvm/llvm-project/releases/download/llvmorg-21.1.8/llvm-project-21.1.8.src.tar.xz \
  "$inputs/llvm-project-21.1.8.src.tar.xz" "$llvm_sha"
status=0
for tc in $toolchains; do
  "$bench" --kache "$kache" --ccache "$ccache" --toolchain "$tc" --out "$out/$tc" \
    --zlib-archive "$inputs/zlib-1.3.2.tar.gz" --llvm-archive "$inputs/llvm-project-21.1.8.src.tar.xz" "$@" || status=1
done
exit $status
