#!/usr/bin/env python3
"""Derive medians from the preserved timing, counter, and perf-stat samples."""
import json
from pathlib import Path
import re
import statistics

root = Path(__file__).parent
report = dict(timing={}, counters={}, stat={})
for directory in ['ablation', 'wake-ablation']:
    data = json.loads((root / directory / 'aiko.json').read_text())
    cells = {}
    for label in sorted({s['variant'] for s in data['samples']}):
        samples = [s for s in data['samples'] if s['variant'] == label]
        rates = [s['requests_per_second'] for s in samples]
        cells[label] = dict(samples=len(samples), requests_per_second=statistics.median(rates),
                            minimum=min(rates), maximum=max(rates),
                            median_per_run_p99_us=statistics.median(s['rtt_p99_us'] for s in samples))
    report['timing'][directory] = cells

worker_names = ['queue_searches', 'local_queue_hits', 'nonblocking_driver_turns', 'idle_driver_turns',
                'sleep_cycles', 'task_polls', 'queue_swaps', 'successful_wake_notifications']
reactor_names = ['driver_turns', 'commands', 'cqes', 'notification_cqes', 'submit_calls', 'wait_calls']
for directory in ['counts', 'counts-fixed']:
    data = json.loads((root / directory / 'aiko.json').read_text())
    for label in sorted({s['variant'] for s in data['samples']}):
        values = {}
        samples = [s for s in data['samples'] if s['variant'] == label]
        for sample in samples:
            text = (root / directory / label / (sample['identifier'] + '.stderr')).read_text()
            counts = {k: json.loads(v) for k, v in re.findall(r'(WORKER_COUNTS|REACTOR_COUNTS) (\[[^\n]+\])', text)}
            for group, names in [('WORKER_COUNTS', worker_names), ('REACTOR_COUNTS', reactor_names)]:
                for name, value in zip(names, counts[group]):
                    values.setdefault(name, []).append(value / sample['requests'])
        report['counters'][label] = dict(samples=len(samples), per_request={k: statistics.median(v) for k, v in values.items()})

for directory in ['stat', 'stat-fixed']:
    data = json.loads((root / directory / 'aiko.json').read_text())
    cells = {}
    for label in sorted({s['variant'] for s in data['samples']}):
        values = {}
        samples = [s for s in data['samples'] if s['variant'] == label]
        for sample in samples:
            for line in (root / directory / label / (sample['identifier'] + '.stat')).read_text().splitlines():
                fields = line.split(';')
                if len(fields) < 3 or fields[0].startswith('#'):
                    continue
                try:
                    value = float(fields[0]) / sample['requests']
                except ValueError:
                    continue
                values.setdefault(fields[2], []).append(value)
        cells[label] = dict(samples=len(samples), per_request={k: statistics.median(v) for k, v in values.items()})
    report['stat'][directory] = cells

(root / 'summary.json').write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps(report, indent=2))
