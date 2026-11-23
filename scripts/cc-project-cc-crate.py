#!/usr/bin/env python3
"""Check cc-rs C and C++ compiles against raw builds through kache."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def exercise(kache, work, cc, cxx, socket_dir):
    work.mkdir(parents=True)
    source = work / 'source'
    shutil.copytree(Path(__file__).resolve().parent.parent / 'test-projects/cc-crate', source)
    env = os.environ.copy()
    env.pop('RUSTC_WRAPPER', None)
    env.update(CARGO_BUILD_RUSTC_WRAPPER='',
               KACHE_CACHE_DIR=str(work / 'cache'),
               KACHE_RUNTIME_DIR=str(work / 'runtime'),
               KACHE_SOCKET_PATH=str(socket_dir / 'daemon.sock'),
               KACHE_CONFIG=str(work / 'kache.toml'),
               KACHE_CC_PATH_NORMALIZE='1', KACHE_CC_SOURCE_DATE_EPOCH='passthrough',
               KACHE_MIN_STORE_COMPILE_MS='0', LC_ALL='C', CFLAGS='-O0 -g0', CXXFLAGS='-O0 -g0')
    Path(env['KACHE_CONFIG']).write_text('')
    log = work / 'build.log'
    rows_seen = 0
    names = {'answer.c', 'adjust.cpp'}

    def run(args, cwd, phase_env=env):
        with log.open('a') as output:
            subprocess.run([str(arg) for arg in args], cwd=cwd, env=phase_env,
                           stdout=output, stderr=subprocess.STDOUT, check=True)

    def stop():
        run([kache, 'daemon', 'stop'], work)

    def edit(value):
        header = source / 'native/answer.h'
        text = header.read_text()
        require(f'#define VALUE {81 - value}' in text, 'unexpected header contents')
        header.write_text(text.replace(f'#define VALUE {81 - value}', f'#define VALUE {value}'))

    def phase(name, value, wrapped, expected=None):
        nonlocal rows_seen
        phase_env = env.copy()
        prefix = str(kache) + ' ' if wrapped else ''
        phase_env.update(CC=prefix + cc, CXX=prefix + cxx,
                         HOST_CC=prefix + cc, HOST_CXX=prefix + cxx)
        if wrapped:
            phase_env['CC_KNOWN_WRAPPER_CUSTOM'] = 'kache'
        else:
            phase_env.pop('CC_KNOWN_WRAPPER_CUSTOM', None)
        run(['cargo', 'clean', '-p', 'cc-crate-fixture'], source, phase_env)
        run(['cargo', 'build', '--locked'], source, phase_env)
        output = subprocess.check_output([str(source / 'target/debug/cc-crate-fixture')],
                                         cwd=source, env=phase_env, text=True)
        require(output == f'{value}\n', f'{name}: wrong program output {output!r}')
        paths = list((source / 'target/debug/build').glob('cc-crate-fixture*/**/out/*.o'))
        require(len(paths) == 2, f'{name}: expected two objects, got {paths}')
        files = {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in paths}
        if expected is not None:
            stop()
            rows = [json.loads(line) for line in (work / 'runtime/events.jsonl').read_text().splitlines()]
            current = [row for row in rows[rows_seen:] if row.get('crate_name') in names]
            rows_seen = len(rows)
            actual = [(row['crate_name'], row['result'], row['compiler_runs']) for row in current]
            require(len(current) == 2 and {row['crate_name'] for row in current} == names and
                    all(row['result'] == expected and row['compiler_runs'] == (0 if expected == 'local_hit' else 1)
                        for row in current), f'{name}: unexpected events {actual}')
        return files

    try:
        raw = phase('raw', 42, False)
        edit(41)
        raw_edit = phase('raw-edit', 44, False)
        require(all(raw[name] != raw_edit[name] for name in raw), 'header edit did not change both objects')
        edit(40)
        cold = phase('cold', 42, True, 'miss')
        require(cold == raw, 'cold C/C++ objects differ from raw')
        print(f'{cc} cold: C and C++ objects byte-identical to raw; two misses; binary 42', flush=True)
        warm = phase('warm', 42, True, 'local_hit')
        require(warm == raw, 'warm C/C++ objects differ from raw')
        print(f'{cc} warm: zero C/C++ compiles, two local hits; binary 42', flush=True)
        edit(41)
        changed = phase('edit', 44, True, 'miss')
        require(changed == raw_edit, 'edited C/C++ objects differ from raw edit')
        print(f'{cc} header edit: exactly two dependent compiles; binary 44', flush=True)
        edit(40)
        reverted = phase('revert', 42, True, 'local_hit')
        require(reverted == raw, 'reverted C/C++ objects differ from raw')
        print(f'{cc} revert: two local hits, zero compiles; binary 42', flush=True)
        relocated = work / 'relocated/source'
        relocated.parent.mkdir()
        shutil.copytree(source, relocated, ignore=shutil.ignore_patterns('target'))
        source = relocated
        moved = phase('relocation', 42, True, 'local_hit')
        require(moved == raw, 'relocated C/C++ objects differ from raw')
        print(f'{cc} relocation: two local hits, zero compiles; binary 42', flush=True)
    finally:
        stop()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('kache', type=Path)
    parser.add_argument('out', type=Path)
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
        with tempfile.TemporaryDirectory(prefix='kache-cc-crate-') as runtime:
            exercise(args.kache.resolve(), run_root / cc, cc, pairs[cc], Path(runtime))


if __name__ == '__main__':
    main()
