#!/usr/bin/env python3
"""Validate complete sample matrices and summarize the completion experiments."""

import argparse
import json
from pathlib import Path
import statistics


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--results", type=Path, default=Path(__file__).with_name("measurements"))
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    paths = sorted(args.results.glob("*/aiko.json")) + sorted(args.results.glob("*.json"))
    result = {}
    clients = set()
    for path in paths:
        report = json.loads(path.read_text())
        if "samples" not in report:
            continue
        assert report["samples"], path
        expected = {(name, repeat, *case) for name in report["variants"] for repeat in range(report["repeats"]) for case in report["cases"]}
        observed = [(sample["variant"], sample["repeat"], sample["workers"], sample["connections"], sample["bytes"]) for sample in report["samples"]]
        assert len(observed) == len(set(observed)) and set(observed) == expected, path
        clients.add(report["client_sha256"])
        name = path.parent.name if path.name == "aiko.json" else path.stem
        result[name] = {"samples": len(observed), "cases": []}
        for case in report["cases"]:
            medians = {}
            for variant in report["variants"]:
                samples = [sample for sample in report["samples"] if sample["variant"] == variant and [sample["workers"], sample["connections"], sample["bytes"]] == case]
                rates = [sample["requests_per_second"] for sample in samples]
                value = {"requests_per_second": statistics.median(rates), "range": [min(rates), max(rates)], "rtt_p99_us": statistics.median(sample["rtt_p99_us"] for sample in samples)}
                if "per_request" in samples[0]:
                    value["per_request"] = {event: statistics.median(sample["per_request"][event] for sample in samples) for event in samples[0]["per_request"]}
                medians[variant] = value
            if "baseline" in medians:
                baseline = medians["baseline"]["requests_per_second"]
                for value in medians.values():
                    value["change_percent"] = 100 * (value["requests_per_second"] / baseline - 1)
            result[name]["cases"].append({"case": case, "variants": medians})
    assert len(clients) == 1, clients
    result["client_sha256"] = clients.pop()
    text = json.dumps(result, indent=2, allow_nan=False) + "\n"
    if args.output:
        args.output.write_text(text)
    else:
        print(text, end="")


if __name__ == "__main__":
    main()
