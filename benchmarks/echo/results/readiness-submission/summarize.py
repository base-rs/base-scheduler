#!/usr/bin/env python3
"""Validate the complete readiness-submission study and print all medians."""
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
build = load('worker-order-build.json')
assert build['source'] == '8400db7da957dfd7dd84de13f210c2b41a98ece1'
assert not build['environment_overrides']
for name, archived in [('candidate.patch', 'worker-order.patch'), ('loop.patch', 'worker-order-loop.patch'), ('submit.patch', 'worker-order-submit.patch')]:
    assert build['patches'][name] == sha(archived)
for variant, patch in [('before', None), ('loop-only', 'worker-order-loop.patch'), ('readiness-submit', 'worker-order.patch')]:
    expected = sha(patch) if patch else hashlib.sha256(b'').hexdigest()
    assert build['binaries'][variant]['source_diff_sha256'] == expected
    assert build['binaries'][variant]['command'] == ['cargo', 'build', '--release', '--locked', '-p', 'core-scheduler-echo-benchmark']

deferred = load('deferred-build.json')
assert deferred['baseline_build'] == build and deferred['source'] == build['source']
assert deferred['patch_sha256'] == sha('deferred.patch')
assert deferred['production_patch_sha256'] == sha('deferred-production.patch')
assert deferred['binaries']['readiness-defer']['source_diff_sha256'] == sha('deferred.patch')
assert all(check['exit_code'] == 0 for check in deferred['verification'])
assert deferred['finished_utc']


def matrix(name, manifest, variants, workloads, repeats):
    data = load(name)
    assert data['build'] == manifest and data['finished_utc']
    assert data['repeats'] == repeats and data['workloads'] == [list(w) for w in workloads]
    assert data['helper']['sha256'] == sha('scripts/sample-runner.py')
    assert data['runner']['sha256'] == sha('scripts/run-driver.py')
    assert data['client']['sha256'] == 'db6c914ab75011477ec6232e681046b1e75532cbaa1c81663bac984591675cf3'
    assert data['warmup_seconds'] == 1 and data['measurement_seconds'] == 3
    cases = {(v, b) for v in variants for b in ['iouring', 'iouring-poll']} | {('control', 'tokio')}
    assert {(c['label'], c['backend']) for c in data['cases']} == cases
    for case in data['cases']:
        label = case['label']
        expected = variants.get(label, build['binaries']['before'])
        assert case['binary']['sha256'] == expected['sha256']
    expected = collections.Counter((v, b, *w, r) for (v, b), w, r in itertools.product(cases, workloads, range(1, repeats + 1)))
    actual = collections.Counter((s['variant'], s['backend'], s['workers'], s['connections'], s['bytes'], s['repeat']) for s in data['samples'])
    assert actual == expected
    print(name, 'completed exchanges/second (full repeat ranges)')
    for workload in workloads:
        for backend in ['iouring', 'iouring-poll', 'tokio']:
            selected = [s for s in data['samples'] if (s['workers'], s['connections'], s['bytes']) == workload and s['backend'] == backend]
            rates = {v: [s['requests_per_second'] for s in selected if s['variant'] == v] for v, b in cases if b == backend}
            reference = median(rates['control' if backend == 'tokio' else 'before'])
            for variant, values in sorted(rates.items()):
                value = median(values)
                print(f'  {workload} {backend} {variant}: {value:,.0f} ({(value/reference-1)*100:+.2f}%), [{min(values):,.0f}–{max(values):,.0f}]')
    return len(data['samples'])


small = [(1, 1, 64), (1, 32, 64), (4, 128, 64)]
workloads = small + [(1, 1, 4096), (4, 128, 4096)]
total = matrix('worker-order-matrix.json', build,
               {v: build['binaries'][b] for v, b in [('before', 'before'), ('loop-only', 'loop-only'), ('candidate', 'readiness-submit')]}, small, 3)
total += matrix('deferred-matrix.json', deferred,
                {'before': build['binaries']['before'], 'candidate': deferred['binaries']['readiness-defer']}, workloads, 3)


def counters(name, manifest, candidate, script, directory):
    data = load(name)
    expected_build = dict(manifest, binaries=dict(manifest['binaries'], before=build['binaries']['before']))
    assert data['build'] == expected_build and data['finished_utc']
    for name in [script, 'sample-runner.py', 'supervise.py']:
        assert data['scripts'][name]['sha256'] == sha('scripts/' + name)
    cases = [('before', 'iouring'), (candidate, 'iouring'), ('before', 'iouring-poll'), (candidate, 'iouring-poll'), ('before', 'tokio')]
    expected = collections.Counter((v, b, r) for (v, b), r in itertools.product(cases, [1, 2]))
    actual = collections.Counter((s['variant'], s['backend'], s['repeat']) for s in data['samples'])
    assert actual == expected
    print('Separate perfstat counters per completed exchange; instrumented throughput is not capacity')
    for variant, backend in cases:
        values = collections.defaultdict(list)
        for sample in data['samples']:
            if (sample['variant'], sample['backend']) != (variant, backend):
                continue
            assert sample['warmup_seconds'] == 0 and sample['seconds'] == 3
            report = root / directory / variant / (sample['identifier'] + '.stat')
            for line in report.read_text().splitlines():
                columns = line.split(';')
                if len(columns) >= 3 and columns[0].replace('.', '', 1).isdigit():
                    values[columns[2]].append(float(columns[0]) / sample['requests'])
        print(f"  {variant} {backend}: enters={median(values['syscalls:sys_enter_io_uring_enter']):.3f}, instructions={median(values['instructions']):,.0f}, cycles={median(values['cycles']):,.0f}")
    return len(data['samples'])


total += counters('deferred-stat.json', deferred, 'readiness-defer', 'stat.py', 'stat')
combined = load('combined-build.json')
assert combined['baseline_build'] == build and combined['source'] == build['source']
assert combined['patch_sha256'] == sha('combined.patch')
assert combined['production_patch_sha256'] == sha('combined-production.patch')
assert combined['binaries']['readiness-drain']['source_diff_sha256'] == sha('combined.patch')
assert combined['script_sha256'] == sha('scripts/build-drain.py')
assert all(check['exit_code'] == 0 for check in combined['verification'])
total += matrix('combined-matrix.json', combined,
                {'before': build['binaries']['before'], 'candidate': combined['binaries']['readiness-drain']}, workloads, 3)
total += counters('combined-stat.json', combined, 'readiness-drain', 'stat-drain.py', 'combined-stat')
verification = load('verification.json')
for label, manifest, regions, tests in [('deferred', deferred, 2141, 82), ('combined', combined, 2152, 84)]:
    record = verification[label]
    assert record['source_patch_sha256'] == manifest['patch_sha256']
    assert record['commands'] == manifest['verification']
    assert record['coverage'] == {'covered_regions': regions, 'measured_regions': regions, 'excluded_unreachable_regions': 1}
    assert len(record['test_results']) == 3
    assert all(f'test result: ok. {n} passed; 0 failed;' in result for n, result in zip([1, tests, 180], record['test_results']))
assert verification['adopted']['patch_sha256'] == sha(verification['adopted']['patch'])
assert total == 233
print(f'Validated {total} samples, patch/source identities, harnesses, and binary identities.')
