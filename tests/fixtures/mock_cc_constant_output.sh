#!/bin/sh
# Keep probe and preprocessor output constant so tests can isolate key inputs.
case "$1" in
  --version) printf 'fake gcc 1.0\n' ;;
  -###) exit 0 ;;
  *)
    source='' dep='' prev=''
    for arg do
      [ "$prev" = -MF ] && dep=$arg
      case "$arg" in *.c|*.S|*.cc|*.cpp) source=$arg ;; esac
      prev=$arg
    done
    [ -n "$dep" ] && printf 'unit.o: %s\n' "$source" > "$dep"
    printf 'preprocessed unit\n' ;;
esac
