#!/usr/bin/env python3
"""Measure pooled receive/parse/slice/send pipelines with real zero-copy reporting."""

import argparse
import hashlib
import itertools
import json
import random
import subprocess
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--repeats', type=int, default=5)
    parser.add_argument('--validation', action='store_true', help='Longer physical-network checks of large local TCP transfers')
    parser.add_argument('--cpus', default='2-5')
    parser.add_argument('--server-host', help='SSH server host when coordinating from another machine')
    parser.add_argument('--server-binary', default='/home/samuel/.cache/core-scheduler-pooled-receivers/target/release/pipeline')
    parser.add_argument('--remote', help='SSH client host for a physical-network comparison')
    parser.add_argument('--address', default='192.168.1.29')
    parser.add_argument('--client-binary', default='/home/samuel/.cache/core-scheduler-pipeline/pipeline')
    args = parser.parse_args()
    binary = args.binary.resolve()
    identity = hashlib.sha256(binary.read_bytes()).hexdigest()
    shapes = [('tcp', size, depth) for size, depth in itertools.product(
        [64, 1200, 16384, 65536, 131072], [1, 8])]
    shapes += [('udp', 1200, depth) for depth in [1, 8]]
    ownerships = ['pooled', 'retained']
    if args.remote:
        shapes = [('tcp', size, depth) for size, depth in itertools.product([1200, 16384, 65536, 131072], [1, 8])]
        shapes += [('udp', 1200, 1)]
        ownerships = ['retained']
    placements = ['local', 'handoff']
    if args.validation:
        assert args.remote, 'validation requires a physical-network client'
        shapes = [('tcp', size, 1) for size in [65536, 131072]]
        placements = ['local']
    groups = list(itertools.product(range(args.repeats), shapes, ownerships, placements))
    randomizer = random.Random(20841)
    randomizer.shuffle(groups)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open('w') as output:
        for repeat, (transport, size, depth), ownership, placement in groups:
            modes = ['copy', 'registered-copy', 'zc', 'fixed-zc']
            randomizer.shuffle(modes)
            iterations = max(1000, min(6000 // depth, 64 * 1024 * 1024 // size // depth))
            if args.remote:
                iterations = max(128, min(2000 // depth, 32 * 1024 * 1024 // size // depth))
            if args.validation:
                iterations = 128 * 1024 * 1024 // size
            for mode in modes:
                command = ['taskset', '-c', args.cpus, str(binary), mode, transport,
                           str(size), str(depth), str(iterations), ownership, placement]
                if args.server_host:
                    command[3] = args.server_binary
                    command = ['ssh', args.server_host, 'flock', '/tmp/core-scheduler-perf.lock'] + command
                if args.remote:
                    server = subprocess.Popen(command + ['0.0.0.0:0'], text=True,
                                              stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                    try:
                        ready = json.loads(server.stdout.readline())
                        port = ready['ready'].rsplit(':', 1)[1]
                        client_command = ['ssh', args.remote, args.client_binary, 'client', transport,
                                          f'{args.address}:{port}', str(size), str(depth), str(iterations)]
                        client = subprocess.run(client_command, text=True, capture_output=True, timeout=90)
                        if client.returncode:
                            raise RuntimeError(f'{client_command}: {client.stderr}')
                        verified = json.loads(client.stdout)
                        assert verified['client_verified']
                        stdout, stderr = server.communicate(timeout=90)
                        if server.returncode:
                            raise RuntimeError(f'{command}: {stderr}')
                    finally:
                        if server.poll() is None:
                            server.kill()
                            server.communicate()
                else:
                    result = subprocess.run(command, text=True, capture_output=True, timeout=90)
                    if result.returncode:
                        raise RuntimeError(f'{command}: {result.stderr}')
                    stdout = result.stdout
                row = json.loads(stdout)
                row.update(repeat=repeat, binary_sha256=identity, cpus=args.cpus,
                           network=args.remote or 'loopback')
                output.write(json.dumps(row) + '\n')
                output.flush()


if __name__ == '__main__':
    main()
