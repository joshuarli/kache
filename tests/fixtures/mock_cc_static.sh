#!/bin/sh
# The dependency file names the source and the first header.h found on the
# -I path, as a compiler including "header.h" would.

case "${1:-}" in
    --version)
        printf '%s\n' 'fake gcc 1.0'
        ;;
    -###)
        exit 0
        ;;
    *)
        source='' dep='' header='' prev=''
        for arg do
            case "$prev" in
                -MF) dep=$arg ;;
                -I) [ -z "$header" ] && [ -f "$arg/header.h" ] && header="$arg/header.h" ;;
            esac
            case "$arg" in *.c|*.S|*.cc|*.cpp) source=$arg ;; esac
            prev=$arg
        done
        [ -n "$dep" ] && printf 'unit.o: %s %s\n' "$source" "$header" > "$dep"
        printf '%s\n' 'preprocessed unit'
        ;;
esac
