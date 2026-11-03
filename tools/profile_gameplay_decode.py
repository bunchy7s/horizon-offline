#!/usr/bin/env python3
"""Profile the validation and typed-deserialization passes in separate processes.

Build `cargo build --release -p horizon-client-lib --example gameplay_decode_profile`
then run this tool with the matching prepared resources. These passes are
both currently required for exact JSON error classification/precedence.
"""
import argparse
import hashlib
import json
import os
import statistics
from pathlib import Path
from profile_shared_json import percentile, run


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path,
                        default=Path("target/release/examples/gameplay_decode_profile"))
    parser.add_argument("--resources", type=Path, default=Path(".local/horizon-client/26.3"))
    parser.add_argument("--repeats", type=int, default=8)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.repeats < 1:
        parser.error("--repeats must be positive")
    binary, resources = args.binary.resolve(), args.resources.resolve()
    if not binary.is_file() or not resources.is_dir():
        parser.error("release example or resources missing")
    modes = ("validate", "typed")
    samples = {mode: [] for mode in modes}
    loads = [os.getloadavg()]
    for i in range(args.repeats):
        for mode in (modes if i % 2 == 0 else reversed(modes)):
            samples[mode].append(run(binary, resources, mode))
    loads.append(os.getloadavg())
    result = {
        "scope": "Isolated gameplay-facts.json validation/typed decoding; includes JSON file read",
        "resources": str(resources),
        "repeats": args.repeats,
        "load_averages_before_after": loads,
        "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "metrics": {
            mode: {
                key: {"median": round(statistics.median(values), 3),
                      "p25": round(percentile(values, .25), 3),
                      "p75": round(percentile(values, .75), 3),
                      "min": round(min(values), 3), "max": round(max(values), 3),
                      "samples": [round(x, 3) for x in values]}
                for key in rows[0]
                for values in [[row[key] for row in rows]]
            }
            for mode, rows in samples.items()
        },
    }
    data = json.dumps(result, indent=2)
    print(data)
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(data + "\n")


if __name__ == "__main__":
    main()
