# Quick screens, 6–7 November 2026

Raw results of the paired screens run on hana with `quick.py` between 6 and 7 November 2026, before the timer benchmark moved to `base-benchmark`. The [performance investigations](../../../../performance/index.md) link the files they rely on. They were copied from `~/Developer/claude-bench/quick/` on hana on 8 November 2026; the source trees and build directories there are not kept.

Host: hana, Intel Xeon E3-1240 v6 (4 cores, 8 threads), Linux 7.2, Rust 1.99.1, release builds, `powersave` governor. Every build and run held `/tmp/base-scheduler-perf.lock`.

| Path | Contents |
| --- | --- |
| [quick.py](quick.py) | The driver: `quick.py echo`, `http`, or `timers`, with BASE and CANDIDATE source trees. It builds both, runs each case with the builds in a random order within each round, and prints the median of per-round changes with the rounds that improved. Unchanged controls (raw Tokio, Hyper) run in the same rounds. |
| [harness/](harness) | The timer diagnostic `quick.py timers` builds against each tree, from [rescue-costs](../../../timers/results/rescue-costs/readme.md). |
| [mw/](mw) | `mw.py`, the multi-worker HTTP driver, with its `[patch]` files, diagnostic and profiling scripts, and results. It builds a private copy of base-http's hello benchmark, with one spawned task per connection, against each tree. |
| [results/](results) | One JSON line per sample, named by mode, topic, and start time. An empty file from an aborted start is omitted. |
| [runnable-counter.patch](runnable-counter.patch) | The rejected per-thread runnable counter, with its revert, from the local branch `perf/runnable-counter`. |
| `selftest-*.txt`, `pr105-rerun.txt` | Printed summaries of the driver's self-tests and of a rerun for #105. |

## The layout control

By default `quick.py` also built BASE from a copy at another path and ran it in the same rounds as a "layout" control, on the assumption that Cargo hashes each package's path into its build. That assumption holds only for some modes:

- **echo:** the control was the same executable. The echo benchmark is a member of the tree's own workspace, and Cargo identifies workspace members by their paths relative to the workspace root, so the copy built byte-identically. For example, `main` and `main-layout` were both `30ed5e22…`, and `main-cd51eee` and its copy both `4560cae1…`. An echo "layout" change therefore measures run-to-run variation of one executable, not build layout. Build layout was not controlled in echo screens.
- **http** and **timers**, and `mw.py`: the control was a different build, a genuine layout sample. These harnesses live outside the tree and depend on it by absolute path, so the copy's path changes their crate hashes; for example `mw.py`'s base, candidate, and copy builds were `88ec9207…`, `8af1fcf3…`, and `6d51c5c2…`.

The investigations state which applies. Comparisons of build layout should now use `base-benchmark`'s layout replicas, which are salted so that they differ.
