project('kache-meson', 'c', 'cpp')
configure_file(output: 'config.h', configuration: {'OFFSET': 2})
lib = static_library('answer', ['answer.c', 'adjust.cpp', 'extra.c'])
executable('check-answer', 'main.cpp', link_with: lib)
