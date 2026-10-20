#!/usr/bin/env python3
"""Validate the archived matrices and print medians; no benchmark is run."""
import collections
import gzip
import hashlib
import itertools
import json
import statistics
from pathlib import Path

root = Path(__file__).resolve().parent
load = lambda name: json.loads((root / name).read_text())
median = statistics.median
sha = lambda data: hashlib.sha256(data).hexdigest()


def matrix(samples, fields, expected):
    cells = collections.Counter(tuple(sample[field] for field in fields) for sample in samples)
    assert cells == collections.Counter(expected), (cells, expected)


def select(samples, **where):
    return [sample for sample in samples if all(sample[key] == value for key, value in where.items())]


def med(samples, field, **where):
    return median(sample[field] for sample in select(samples, **where))


def change(before, after):
    return (after / before - 1) * 100


build = load('timer-build.json')
assert build['patches']['core-scheduler-timer-flush-production.patch'] == sha((root / 'timer-flush.patch').read_bytes())
assert build['patches']['core-scheduler-timer-local-pop.patch'] == sha((root / 'local-pop.patch').read_bytes())
assert build['patches']['candidate.patch'] == sha((root.parent / 'local-timers/candidate.patch').read_bytes())
assert build['harness_sha256'] == sha((root.parent / 'timer-ready/scripts/timers.rs').read_bytes())
micro = load('timer-micro.json')
assert micro['harness_sha256'] == sha((root / 'scripts/queue_perf.rs').read_bytes())
expected = [(variant, queue, workload, repeat)
            for variant, queue in [('production', 'shared'), ('flush', 'shared'), ('local', 'local'), ('local-fast', 'local')]
            for workload in ['deadline', 'expiry'] for repeat in range(1, 10)]
matrix(micro['samples'], ['variant', 'queue', 'workload', 'repeat'], expected)
print('Timer primitive: median milliseconds (9 repetitions)')
for before, after in [('production', 'flush'), ('local', 'local-fast')]:
    for workload in ['deadline', 'expiry']:
        a, b = [med(micro['samples'], 'elapsed_ns', variant=v, workload=workload) / 1e6 for v in [before, after]]
        print(f'  {before}->{after} {workload}: {a:.3f}->{b:.3f} ({change(a,b):+.2f}%)')

timers = load('timers/results.json')
assert timers['build'] == build
workloads = [(1, 1, 10, False), (4, 1024, 1000, False), (4, 1024, 1000, True)]
matrix(timers['samples'], ['variant', 'workers', 'tasks', 'delay_us', 'aligned', 'repeat'],
       [(v, *w, r) for v in build['variants'] for w in workloads for r in range(1, 4)])
print('Whole scheduler: median process CPU seconds / expiries per second / p99 lateness us')
for workers, tasks, delay, aligned in workloads:
    print(f'  workers={workers} timers={tasks} delay_us={delay} aligned={aligned}')
    for v in build['variants']:
        samples = select(timers['samples'], variant=v, workers=workers, tasks=tasks, delay_us=delay, aligned=aligned)
        cpu = median(s['user_seconds'] + s['system_seconds'] for s in samples)
        print(f"    {v}: {cpu:.6f} / {median(s['expiries_per_second'] for s in samples):,.0f} / {median(s['lateness_p99_us'] for s in samples):.3f}")

profile = load('timer-loop-profile.json')
matrix(profile['samples'], ['mode', 'variant', 'backend', 'repeat'],
       [(m, v, b, r) for m, repeats in [('control', 3), ('stat', 2), ('perf', 2)]
        for v in ['before', 'local'] for b in ['iouring', 'iouring-poll'] for r in range(1, repeats + 1)])
for name, info in load('profile-text-normalization.json').items():
    compressed = (root / info['compressed_path']).read_bytes()
    assert sha(compressed) == info['compressed_sha256']
    assert sha(gzip.decompress(compressed)) == info['archived_sha256']
print('No-timer capacity controls: exchanges/second (3 repetitions)')
for b in ['iouring', 'iouring-poll']:
    a, c = [med(profile['samples'], 'requests_per_second', mode='control', variant=v, backend=b) for v in ['before', 'local']]
    print(f'  {b}: {a:,.0f}->{c:,.0f} ({change(a,c):+.2f}%)')
print('Diagnostic counts per completed exchange (2 perfstat repetitions; not capacity samples)')
for b in ['iouring', 'iouring-poll']:
    for v in ['before', 'local']:
        ratios = collections.defaultdict(list)
        for sample in select(profile['samples'], mode='stat', variant=v, backend=b):
            text = (root / 'timer-loop' / v / 'stat' / (sample['identifier'] + '.stat')).read_text()
            for line in text.splitlines():
                columns = line.split(';')
                if len(columns) >= 3 and columns[0].replace('.', '', 1).isdigit():
                    ratios[columns[2]].append(float(columns[0]) / sample['requests'])
        print(f"  {v} {b}: instructions={median(ratios['instructions']):,.0f}, cycles={median(ratios['cycles']):,.0f}, ring_enters={median(ratios['syscalls:sys_enter_io_uring_enter']):.3f}, context_switches={median(ratios['context-switches']):.3f}")
print('Validated 136 samples and compressed profile checksums.')
