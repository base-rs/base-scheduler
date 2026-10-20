#!/usr/bin/env python3
"""Validate separately timed driver variants and print capacity/counter tables."""
import collections
import hashlib
import itertools
import json
import statistics
from pathlib import Path

root = Path(__file__).resolve().parent
load = lambda name: json.loads((root / name).read_text())
sha = lambda name: hashlib.sha256((root / name).read_bytes()).hexdigest()
median = statistics.median
build = load('driver-build.json')
assert build['common_patch_sha256'] == sha('common-hotpath.patch')
assert build['patch_sha256'] == sha('driver-turn-prepared.patch')
cases = {('before', 'iouring'), ('candidate', 'iouring'), ('before', 'iouring-poll'), ('candidate', 'iouring-poll'), ('control', 'tokio')}
total = 0
for name, workloads, repeats in [
    ('driver-matrix.json', [(1, 1, 64), (1, 32, 64), (4, 128, 64)], 3),
    ('driver-confirm.json', [(1, 1, 64), (1, 1, 4096), (4, 128, 4096)], 3),
    ('driver-concurrent.json', [(1, 32, 64)], 5),
]:
    data = load(name)
    assert data['build'] == build and data['finished_utc']
    assert data['repeats'] == repeats and data['workloads'] == [list(w) for w in workloads]
    assert data['helper']['sha256'] == sha('scripts/sample-runner.py')
    assert data['runner']['sha256'] == sha('scripts/run-driver.py')
    assert {(c['label'], c['backend']) for c in data['cases']} == cases
    for case in data['cases']:
        variant = 'driver-turn' if case['label'] == 'candidate' else 'before'
        assert case['binary']['sha256'] == build['binaries'][variant]['sha256']
    expected = {(v, b, *w, r) for (v, b), w, r in itertools.product(cases, workloads, range(1, repeats + 1))}
    actual = [(s['variant'], s['backend'], s['workers'], s['connections'], s['bytes'], s['repeat']) for s in data['samples']]
    assert len(actual) == len(expected) and set(actual) == expected
    total += len(actual)
    print(name, 'completed exchanges/second')
    for w in workloads:
        for backend in ['iouring', 'iouring-poll', 'tokio']:
            selected = [s for s in data['samples'] if (s['workers'], s['connections'], s['bytes']) == w and s['backend'] == backend]
            rates = {v: [s['requests_per_second'] for s in selected if s['variant'] == v] for v, b in cases if b == backend}
            if backend == 'tokio':
                print(f"  {w} Tokio control: {median(rates['control']):,.0f}")
            else:
                a, b = [median(rates[v]) for v in ['before', 'candidate']]
                print(f'  {w} {backend}: {a:,.0f}->{b:,.0f} ({(b/a-1)*100:+.2f}%)')

data = load('driver-stat.json')
assert data['build'] == build and data['finished_utc']
expected = {(v, b, r) for v, b in [('before', 'iouring'), ('driver-turn', 'iouring'), ('before', 'iouring-poll'), ('driver-turn', 'iouring-poll'), ('before', 'tokio')] for r in [1, 2]}
actual = [(s['variant'], s['backend'], s['repeat']) for s in data['samples']]
assert len(actual) == len(expected) and set(actual) == expected
total += len(actual)
print('Diagnostic perfstat per completed exchange (not capacity samples)')
for v, b in sorted({(v, b) for v, b, _ in expected}):
    counters = collections.defaultdict(list)
    for sample in data['samples']:
        if (sample['variant'], sample['backend']) != (v, b):
            continue
        assert sample['warmup_seconds'] == 0
        text = (root / 'driver-stat' / v / (sample['identifier'] + '.stat')).read_text()
        for line in text.splitlines():
            columns = line.split(';')
            if len(columns) >= 3 and columns[0].replace('.', '', 1).isdigit():
                counters[columns[2]].append(float(columns[0]) / sample['requests'])
    print(f"  {v} {b}: ring_enters={median(counters['syscalls:sys_enter_io_uring_enter']):.3f}, instructions={median(counters['instructions']):,.0f}, cycles={median(counters['cycles']):,.0f}")

drain = load('driver-drain-build.json')
assert drain['initial_build'] == build
assert drain['incremental_patch_sha256'] == sha('driver-wait-drain.patch')
data = load('driver-drain-matrix.json')
assert data['build'] == drain and data['finished_utc']
workloads = [(1, 1, 64), (1, 32, 64), (4, 128, 64), (1, 1, 4096), (4, 128, 4096)]
assert data['workloads'] == [list(w) for w in workloads] and data['repeats'] == 3
assert data['helper']['sha256'] == sha('scripts/sample-runner.py')
assert data['runner']['sha256'] == sha('scripts/run-driver.py')
assert {(c['label'], c['backend']) for c in data['cases']} == cases
for case in data['cases']:
    expected_hash = drain['binaries']['driver-drain']['sha256'] if case['label'] == 'candidate' else build['binaries']['before']['sha256']
    assert case['binary']['sha256'] == expected_hash
expected = {(v, b, *w, r) for (v, b), w, r in itertools.product(cases, workloads, range(1, 4))}
actual = [(s['variant'], s['backend'], s['workers'], s['connections'], s['bytes'], s['repeat']) for s in data['samples']]
assert len(actual) == len(expected) and set(actual) == expected
total += len(actual)
print('Combined wait plus same-turn drain: completed exchanges/second')
for w in workloads:
    for backend in ['iouring', 'iouring-poll', 'tokio']:
        selected = [s for s in data['samples'] if (s['workers'], s['connections'], s['bytes']) == w and s['backend'] == backend]
        rates = {v: [s['requests_per_second'] for s in selected if s['variant'] == v] for v, b in cases if b == backend}
        if backend == 'tokio':
            print(f"  {w} Tokio control: {median(rates['control']):,.0f}")
        else:
            a, b = [median(rates[v]) for v in ['before', 'candidate']]
            print(f'  {w} {backend}: {a:,.0f}->{b:,.0f} ({(b/a-1)*100:+.2f}%)')

data = load('driver-drain-stat.json')
assert data['build'] == drain and data['finished_utc']
expected = {(v, b, r) for v, b in [('before', 'iouring'), ('driver-drain', 'iouring'), ('before', 'iouring-poll'), ('driver-drain', 'iouring-poll'), ('before', 'tokio')] for r in [1, 2]}
actual = [(s['variant'], s['backend'], s['repeat']) for s in data['samples']]
assert len(actual) == len(expected) and set(actual) == expected
total += len(actual)
print('Combined wait/drain diagnostic perfstat per exchange')
for v, b in sorted({(v, b) for v, b, _ in expected}):
    counters = collections.defaultdict(list)
    for sample in data['samples']:
        if (sample['variant'], sample['backend']) != (v, b):
            continue
        assert sample['warmup_seconds'] == 0
        text = (root / 'driver-drain-stat' / v / (sample['identifier'] + '.stat')).read_text()
        for line in text.splitlines():
            columns = line.split(';')
            if len(columns) >= 3 and columns[0].replace('.', '', 1).isdigit():
                counters[columns[2]].append(float(columns[0]) / sample['requests'])
    print(f"  {v} {b}: ring_enters={median(counters['syscalls:sys_enter_io_uring_enter']):.3f}, instructions={median(counters['instructions']):,.0f}, cycles={median(counters['cycles']):,.0f}")
minimal = load('wait-drain-only-build.json')
assert minimal['initial_build'] == build
assert minimal['variant_patch_sha256'] == sha('wait-drain-only.patch')
sections = []
for name in ['common-hotpath.patch', 'wait-drain-only.patch']:
    sections.extend(b'diff --git ' + part for part in (root / name).read_bytes().split(b'diff --git ')[1:])
source_diff = b''.join(sorted(sections, key=lambda part: part.split(b'\n', 1)[0]))
assert hashlib.sha256(source_diff).hexdigest() == minimal['source_diff_sha256']
data = load('wait-drain-only-matrix.json')
assert data['build'] == minimal and data['finished_utc']
assert data['workloads'] == [list(w) for w in workloads] and data['repeats'] == 3
assert data['helper']['sha256'] == sha('scripts/sample-runner.py')
assert data['runner']['sha256'] == sha('scripts/run-driver.py')
assert data['client']['sha256'] == 'db6c914ab75011477ec6232e681046b1e75532cbaa1c81663bac984591675cf3'
assert data['warmup_seconds'] == 1 and data['measurement_seconds'] == 3
assert {(c['label'], c['backend']) for c in data['cases']} == cases
for case in data['cases']:
    variant = minimal['binaries']['wait-drain-only'] if case['label'] == 'candidate' else build['binaries']['before']
    assert case['binary']['sha256'] == variant['sha256']
expected = {(v, b, *w, r) for (v, b), w, r in itertools.product(cases, workloads, range(1, 4))}
actual = [(s['variant'], s['backend'], s['workers'], s['connections'], s['bytes'], s['repeat']) for s in data['samples']]
assert len(actual) == len(expected) and set(actual) == expected
total += len(actual)
print('Post-wait collection only: completed exchanges/second (full repeat ranges)')
for w in workloads:
    for backend in ['iouring', 'iouring-poll', 'tokio']:
        selected = [s for s in data['samples'] if (s['workers'], s['connections'], s['bytes']) == w and s['backend'] == backend]
        rates = {v: [s['requests_per_second'] for s in selected if s['variant'] == v] for v, b in cases if b == backend}
        if backend == 'tokio':
            r = rates['control']
            print(f"  {w} Tokio control: {median(r):,.0f} [{min(r):,.0f}–{max(r):,.0f}]")
        else:
            a, b = [median(rates[v]) for v in ['before', 'candidate']]
            ranges = ', '.join(f'{v} [{min(rates[v]):,.0f}–{max(rates[v]):,.0f}]' for v in ['before', 'candidate'])
            print(f'  {w} {backend}: {a:,.0f}->{b:,.0f} ({(b/a-1)*100:+.2f}%), {ranges}')
print(f'Validated {total} driver samples.')
