#!/usr/bin/env python3
"""Archive results without perf binaries; retain exact decoded report contents."""

import argparse
import gzip
import hashlib
import json
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    manifest = {}
    for path in sorted(args.source.rglob("*")):
        if not path.is_file() or path.suffix not in {".json", ".txt", ".csv", ".stat", ".stderr", ".strace"}:
            continue
        data = path.read_bytes()
        if not data:
            continue
        relative = path.relative_to(args.source)
        target = args.output / relative
        original = hashlib.sha256(data).hexdigest()
        encoding = "original"
        if path.suffix == ".json":
            value = json.loads(data)
            if isinstance(value, dict) and "samples" in value:
                metadata = {key: entry for key, entry in value.items() if key != "samples"}
                prefix = json.dumps(metadata, indent=2, allow_nan=False)[:-2]
                samples = ",\n".join("    " + json.dumps(sample, allow_nan=False, separators=(",", ":")) for sample in value["samples"])
                data = (prefix + ',\n  "samples": [\n' + samples + '\n  ]\n}\n').encode()
                assert json.loads(data) == value
                encoding = "compact samples, unchanged parsed JSON"
        elif path.suffix in {".txt", ".strace"}:
            data = gzip.compress(data, mtime=0)
            target = target.with_name(target.name + ".gz")
            encoding = "gzip"
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
        manifest[str(relative)] = {"path": str(target.relative_to(args.output)), "source_sha256": original, "archived_sha256": hashlib.sha256(data).hexdigest(), "encoding": encoding}
    (args.output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")


if __name__ == "__main__":
    main()
