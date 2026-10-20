#!/usr/bin/env python3
"""Validate the archived worker hot-path samples and regenerate their tables."""
import hashlib
import itertools
import json
import statistics
from pathlib import Path

ROOT = Path(__file__).resolve().parent
LOADS = [(1, 1, 64), (1, 32, 64), (4, 128, 64)]
BACKENDS = ['iouring', 'iouring-poll', 'tokio']


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def worker_change(path):
    patch = path.read_text()
    marker = 'diff --git a/crates/native/src/worker.rs '
    start = patch.index(marker)
    rest = patch[start:]
    return rest.split('\ndiff --git ', 1)[0]


def summarize(directory, variant, repeats, loads, cases):
    data = json.loads((ROOT / directory / 'aiko.json').read_text())
    manifest_name = 'build.json' if variant == 'single-local' else 'build-injector-empty.json'
    manifest = json.loads((ROOT / manifest_name).read_text())
    assert data['build'] == manifest, directory
    assert data['repeats'] == repeats and data['workloads'] == [list(load) for load in loads]
    assert data['warmup_seconds'] == 1 and data['measurement_seconds'] == 3
    assert data['helper']['sha256'] == digest(ROOT / 'sample-runner.py')
    assert data['runner']['sha256'] == digest(ROOT / 'run.py')
    assert manifest['patch']['sha256'] == digest(ROOT / 'single-local.patch')
    assert worker_change(ROOT / f'{variant}.patch') == worker_change(ROOT / f'{variant}-reviewed.patch')
    if variant == 'injector-empty':
        assert manifest['variant_patch_sha256'] == digest(ROOT / 'injector-empty.patch')
    assert {(case['label'], case['backend']) for case in data['cases']} == set(cases)
    for case in data['cases']:
        label = case['label']
        expected_hash = (
            manifest['variant_binary_sha256']
            if label == 'injector-empty'
            else manifest['binaries'][label]['sha256']
        )
        assert case['binary']['sha256'] == expected_hash, case
    expected = {
        (label, backend, load, repeat)
        for (label, backend), load, repeat in itertools.product(cases, loads, range(1, repeats + 1))
    }
    samples = data['samples']
    actual = [
        (sample['variant'], sample['backend'],
         (sample['workers'], sample['connections'], sample['bytes']), sample['repeat'])
        for sample in samples
    ]
    assert len(actual) == len(expected) and set(actual) == expected, directory
    rows = []
    for workers, connections, size in loads:
        row = {'workers': workers, 'connections': connections, 'bytes': size, 'backends': {}}
        for backend in BACKENDS:
            cell = {}
            for label in ['before', variant]:
                selected = [
                    sample for sample in samples
                    if (sample['variant'], sample['backend'], sample['workers'], sample['connections'], sample['bytes'])
                    == (label, backend, workers, connections, size)
                ]
                if not selected:
                    continue
                rates = [sample['requests_per_second'] for sample in selected]
                assert all(sample['requests'] > 0 and sample['seconds'] == 3 for sample in selected)
                cell[label] = {
                    'median': statistics.median(rates), 'minimum': min(rates), 'maximum': max(rates),
                    'server_cpu_seconds': statistics.median(
                        sample['server_cpu']['user'] + sample['server_cpu']['system'] for sample in selected
                    ),
                    'rtt_p99_us': statistics.median(sample['rtt_p99_us'] for sample in selected),
                }
            if variant in cell:
                cell['change_percent'] = (cell[variant]['median'] / cell['before']['median'] - 1) * 100
            row['backends'][backend] = cell
        rows.append(row)
    return {'sample_count': len(samples), 'rows': rows, 'cases': data['cases']}


reports = {}
lines = [
    '# Worker hot-path measurements', '',
    'All rates are median completed echo exchanges per second.', '',
    '| Experiment | Workers/connections | Owned before → candidate | Change | Borrowed before → candidate | Change | Tokio before → candidate control | Change |',
    '| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |',
]
for directory, variant in [('echo-results', 'single-local'), ('injector-results', 'injector-empty')]:
    cases = list(itertools.product(['before', variant], BACKENDS))
    report = summarize(directory, variant, 3, LOADS, cases)
    reports[variant] = report
    for row in report['rows']:
        cells = []
        for backend in BACKENDS:
            cell = row['backends'][backend]
            cells.extend([
                f"{cell['before']['median']:,.0f} → {cell[variant]['median']:,.0f}",
                f"{cell['change_percent']:+.2f}%",
            ])
        lines.append(f"| {variant} | {row['workers']}/{row['connections']} | " + ' | '.join(cells) + ' |')

confirmation = ROOT / 'injector-confirmation' / 'aiko.json'
if confirmation.exists():
    variant = 'injector-empty'
    cases = [('before', 'iouring'), (variant, 'iouring'), ('before', 'iouring-poll'),
             (variant, 'iouring-poll'), ('before', 'tokio')]
    report = summarize('injector-confirmation', variant, 5, [(1, 1, 64)], cases)
    reports['injector-confirmation'] = report
    lines.extend(['', '## Five-repetition single-connection confirmation', '',
                  '| Backend | Before | Candidate | Change |', '| --- | ---: | ---: | ---: |'])
    for backend in BACKENDS:
        cell = report['rows'][0]['backends'][backend]
        if variant in cell:
            lines.append(f"| {backend} | {cell['before']['median']:,.0f} | {cell[variant]['median']:,.0f} | {cell['change_percent']:+.2f}% |")
        else:
            lines.append(f"| Unchanged Tokio control | {cell['before']['median']:,.0f} | — | — |")

(ROOT / 'summary.json').write_text(json.dumps(reports, indent=2) + '\n')
(ROOT / 'tables.md').write_text('\n'.join(lines) + '\n')
print(f"Validated {sum(report['sample_count'] for report in reports.values())} samples.")
print('\n'.join(lines))
