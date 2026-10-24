#!/usr/bin/env python3
"""Compare Native worker policies across borrowed and owned echo paths."""

import argparse
import hashlib
import importlib.util
import itertools
import json
from pathlib import Path
import platform
import random
import subprocess
from types import SimpleNamespace


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--variant", action="append", required=True, help="NAME=BINARY")
    parser.add_argument("--tokio", type=Path, required=True)
    parser.add_argument("--client", type=Path, required=True)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--cases", nargs="+", default=[f"{w}:{c}:{b}" for w,c,b in itertools.product([1,4],[1,32],[64,4096])])
    parser.add_argument("--repeats", type=int, default=3)
    parser.add_argument("--paths", nargs="+", choices=["borrowed","owned"], default=["borrowed","owned"])
    args = parser.parse_args()
    if args.output.exists():
        parser.error("output already exists")
    variants = dict(item.split("=", 1) for item in args.variant)
    if len(variants) != len(args.variant) or "tokio" in variants:
        parser.error("duplicate or reserved variant name")
    spec = importlib.util.spec_from_file_location("echo_run", args.root / "benchmarks/echo/run.py")
    run = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(run)
    variants = {name: Path(binary).resolve() for name,binary in variants.items()}
    settings = SimpleNamespace(server_cpus=[0,1,2,3], client_cpus=[4,5,6,7], client_binary=args.client.resolve(), client_workers=4, warmup=1, seconds=3)
    cases = [tuple(map(int,case.split(":"))) for case in args.cases]
    report = {
        "platform": platform.platform(), "hostname": platform.node(),
        "rustc": subprocess.check_output(["rustc", "-Vv"],text=True),
        "variants": {name:{"binary":str(binary),"sha256":hashlib.sha256(binary.read_bytes()).hexdigest()} for name,binary in (variants | {"tokio":args.tokio.resolve()}).items()},
        "settings":vars(settings) | {"client_binary":str(settings.client_binary)},
        "client_sha256":hashlib.sha256(settings.client_binary.read_bytes()).hexdigest(),
        "cases":cases, "paths":{"borrowed":"iouring-monitor","owned":"iouring","common":"tokio"},
        "repeats":args.repeats, "selected_paths":args.paths, "seed":20261025, "samples":[],
    }
    hashes = [report["variants"][name]["sha256"] for name in variants]
    if len(set(hashes)) != len(hashes):
        parser.error("duplicate Native executable hash")
    jobs = [(r,c,name,path) for r,c,name,path in itertools.product(range(args.repeats),cases,variants,args.paths)]
    jobs += [(r,c,"tokio","common") for r,c in itertools.product(range(args.repeats),cases)]
    random.Random(report["seed"]).shuffle(jobs)
    for repeat,(workers,connections,size),name,path in jobs:
        binary = args.tokio.resolve() if name == "tokio" else variants[name]
        sample = run.sample(binary,report["paths"][path],workers,connections,size,settings)
        sample.update(variant=name,path=path,repeat=repeat)
        report["samples"].append(sample)
        args.output.write_text(json.dumps(report,indent=2)+"\n")
        print(name,path,workers,connections,size,repeat,round(sample["requests_per_second"]),flush=True)


if __name__ == "__main__":
    main()
