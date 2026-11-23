#!/usr/bin/env python3
"""Check Meson and Ninja compiler launchers against raw C and C++ builds."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile
import time


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def exercise(kache, gluon, work, cc, cxx, socket_dir):
    work.mkdir(parents=True)
    source = work / 'tree/source'
    shutil.copytree(Path(__file__).resolve().parent.parent / 'test-projects/cc-meson', source)
    build = source.parent / 'build'
    env = os.environ.copy()
    env.pop('RUSTC_WRAPPER', None)
    env.update(KACHE_CACHE_DIR=str(work / 'cache'),
               KACHE_RUNTIME_DIR=str(work / 'runtime'),
               KACHE_SOCKET_PATH=str(socket_dir / 'daemon.sock'),
               KACHE_CONFIG=str(work / 'kache.toml'),
               KACHE_CC_PATH_NORMALIZE='1', KACHE_CC_SOURCE_DATE_EPOCH='passthrough',
               KACHE_MIN_STORE_COMPILE_MS='0', LC_ALL='C')
    Path(env['KACHE_CONFIG']).write_text('')
    log = work / 'build.log'
    rows_seen = 0
    names = {'answer.c', 'adjust.cpp', 'extra.c', 'main.cpp'}

    def run(args, cwd, phase_env=env):
        with log.open('a') as output:
            subprocess.run([str(arg) for arg in args], cwd=cwd, env=phase_env,
                           stdout=output, stderr=subprocess.STDOUT, check=True)

    def stop():
        run([kache, 'daemon', 'stop'], work)

    def configure(wrapped):
        phase_env = env.copy()
        prefix = shlex.quote(str(kache)) + ' ' if wrapped else ''
        phase_env.update(CC=prefix + cc, CXX=prefix + cxx, CFLAGS='-O0 -g0', CXXFLAGS='-O0 -g0')
        run([gluon / 'meson', 'setup', build, source], work, phase_env)

    def edit(value):
        header = source / 'answer.h'
        header.write_text(header.read_text().replace(f'#define VALUE {81 - value}', f'#define VALUE {value}'))
        time.sleep(2.1)

    def phase(name, expected_output, expected_events=None, clean=False):
        nonlocal rows_seen
        if clean:
            run([gluon / 'ninja', '-C', build, '-t', 'clean'], work)
        run([gluon / 'ninja', '-C', build], work)
        output = subprocess.check_output([str(build / 'check-answer')], cwd=build, env=env, text=True)
        require(output == f'{expected_output}\n', f'{name}: wrong program output {output!r}')
        objects = sorted(build.rglob('*.o'))
        require(len(objects) == 4, f'{name}: expected four objects, got {objects}')
        files = {str(p.relative_to(build)): hashlib.sha256(p.read_bytes()).hexdigest() for p in objects}
        if expected_events is not None:
            stop()
            events = work / 'runtime/events.jsonl'
            rows = [json.loads(line) for line in events.read_text().splitlines()]
            current = [row for row in rows[rows_seen:] if row.get('crate_name') in names]
            rows_seen = len(rows)
            actual = [(row['crate_name'], row['result'], row['compiler_runs']) for row in current]
            require(len(current) == len(expected_events) and
                    {row['crate_name'] for row in current} == set(expected_events) and
                    all(row['result'] == expected_events[row['crate_name']] for row in current),
                    f'{name}: unexpected events {actual}, expected {expected_events}')
            require(all(row['compiler_runs'] == (0 if row['result'] == 'local_hit' else 1)
                        for row in current), f'{name}: unexpected compiler runs: {actual}')
        return files

    try:
        configure(False)
        raw = phase('raw', 42)
        edit(41)
        raw_edit = phase('raw-edit', 44)
        require(raw_edit != raw, 'edited header did not change objects')
        edit(40)
        shutil.rmtree(build)
        configure(True)
        cold = phase('cold', 42, dict.fromkeys(names, 'miss'))
        require(cold == raw, 'cold objects differ from raw')
        print(f'{cc} raw and cold: four byte-identical objects; cold: four misses', flush=True)
        warm = phase('warm', 42, dict.fromkeys(names, 'local_hit'), clean=True)
        require(warm == raw, 'warm objects differ from raw')
        print(f'{cc} warm: four byte-identical objects; zero compiles, four local hits', flush=True)
        edit(41)
        changed = phase('edit', 44, dict.fromkeys({'answer.c', 'adjust.cpp'}, 'miss'))
        require(changed == raw_edit, 'edited objects differ from raw edit')
        print(f'{cc} edit: four byte-identical objects; exactly two dependent compiles', flush=True)
        edit(40)
        reverted = phase('revert', 42, dict.fromkeys({'answer.c', 'adjust.cpp'}, 'local_hit'))
        require(reverted == raw, 'reverted objects differ from raw')
        print(f'{cc} revert: four byte-identical objects; two local hits, zero compiles', flush=True)
        relocated = work / 'relocated'
        source = relocated / 'source'
        shutil.copytree(work / 'tree/source', source)
        build = relocated / 'build'
        configure(True)
        moved = phase('relocation', 42, dict.fromkeys(names, 'local_hit'))
        require(moved == raw, 'relocated objects differ from raw')
        print(f'{cc} relocation: four byte-identical objects; four local hits, zero compiles', flush=True)
    finally:
        stop()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('kache', type=Path)
    parser.add_argument('out', type=Path)
    parser.add_argument('gluon', type=Path)
    parser.add_argument('toolchains', nargs='+')
    args = parser.parse_args()
    pairs = {'gcc': 'g++', 'clang': 'clang++', 'cc': 'c++'}
    for cc in args.toolchains:
        require(cc in pairs and shutil.which(cc) and shutil.which(pairs[cc]), f'unknown or missing toolchain: {cc}')
    root = args.out.resolve()
    root.mkdir(parents=True, exist_ok=True)
    run_root = Path(tempfile.mkdtemp(prefix='run-', dir=root))
    print(f'Evidence: {run_root}', flush=True)
    for cc in args.toolchains:
        with tempfile.TemporaryDirectory(prefix='kache-meson-') as runtime:
            exercise(args.kache.resolve(), args.gluon.resolve(), run_root / cc, cc, pairs[cc], Path(runtime))


if __name__ == '__main__':
    main()
