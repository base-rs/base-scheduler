#!/usr/bin/env python3
"""Validate paired pipeline measurements and report median per-repeat ratios."""

import argparse
import collections
import json
import statistics
from pathlib import Path

MODES = {'copy', 'registered-copy', 'zc', 'fixed-zc'}


def summarize(path):
    rows = [json.loads(line) for line in path.read_text().splitlines()]
    assert rows and len({row['binary_sha256'] for row in rows}) == 1
    pairs = collections.defaultdict(dict)
    for row in rows:
        assert row['bytes'] == (row['size'] - 16) * row['depth'] * row['iterations']
        assert row['sends'] >= row['depth'] * row['iterations']
        assert 0 <= row['copied_notifications'] <= row['notifications'] <= row['sends']
        if row['mode'] in ['copy', 'registered-copy']:
            assert row['notifications'] == 0
        key = tuple(row[field] for field in ['network', 'transport', 'size', 'depth', 'ownership', 'placement', 'repeat'])
        assert row['mode'] not in pairs[key]
        pairs[key][row['mode']] = row
    cells = collections.defaultdict(list)
    for key, modes in pairs.items():
        assert set(modes) == MODES, (key, set(modes))
        cells[key[:-1]].append(modes)
    result = []
    for key, repeats in sorted(cells.items()):
        cell = dict(zip(['network', 'transport', 'size', 'depth', 'ownership', 'placement'], key))
        cell['repetitions'] = len(repeats)
        cell['modes'] = {}
        baseline = lambda pair: pair['copy']['messages_per_second']
        cpu = lambda row: row['cpu_user_seconds'] + row['cpu_system_seconds']
        for mode in sorted(MODES):
            ratios = [pair[mode]['messages_per_second'] / baseline(pair) for pair in repeats]
            cpu_ratios = [cpu(pair[mode]) / cpu(pair['copy']) for pair in repeats]
            records = [pair[mode] for pair in repeats]
            cell['modes'][mode] = dict(
                throughput_ratio=statistics.median(ratios),
                throughput_ratio_range=[min(ratios), max(ratios)],
                cpu_ratio=statistics.median(cpu_ratios),
                cpu_ratio_range=[min(cpu_ratios), max(cpu_ratios)],
                gib_per_second=statistics.median(row['gib_per_second'] for row in records),
                notifications=sum(row['notifications'] for row in records),
                copied_notifications=sum(row['copied_notifications'] for row in records),
                copied_bytes=sum(row['copied_bytes'] for row in records),
                sends=sum(row['sends'] for row in records),
                pool_stalls=sum(row['pool_stalls'] for row in records),
            )
        cell['fixed_vs_unregistered_zc'] = statistics.median(
            pair['fixed-zc']['messages_per_second'] / pair['zc']['messages_per_second'] for pair in repeats)
        result.append(cell)
    return dict(samples=len(rows), binary_sha256=rows[0]['binary_sha256'], cells=result)


def markdown(result):
    lines = ['# Receive/parse/slice/send pipeline measurements', '',
             f"{result['samples']} optimized samples. Ratios are medians of paired per-repeat throughput; CPU ratios compare total process user + system time for the same byte count.", '',
             '| Transport | Bytes | Connections | Ownership | Placement | Registered copy | ZC | Fixed ZC | Fixed ZC CPU | Fixed / ZC |',
             '|---|---:|---:|---|---|---:|---:|---:|---:|---:|']
    for cell in result['cells']:
        modes = cell['modes']
        lines.append(f"| {cell['transport']} | {cell['size']} | {cell['depth']} | {cell['ownership']} | {cell['placement']} | "
                     f"{modes['registered-copy']['throughput_ratio']:.3f}× | {modes['zc']['throughput_ratio']:.3f}× | "
                     f"{modes['fixed-zc']['throughput_ratio']:.3f}× | {modes['fixed-zc']['cpu_ratio']:.3f}× | {cell['fixed_vs_unregistered_zc']:.3f}× |")
    lines += ['', 'The JSON summary retains ranges, absolute throughput, usage notifications, and pool stalls. “Registered copy” registers the same storage but uses ordinary send operations.']
    return '\n'.join(lines) + '\n'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('input', type=Path)
    parser.add_argument('--json', type=Path, required=True)
    parser.add_argument('--markdown', type=Path, required=True)
    args = parser.parse_args()
    result = summarize(args.input)
    args.json.write_text(json.dumps(result, indent=2) + '\n')
    args.markdown.write_text(markdown(result))


if __name__ == '__main__':
    main()
