# Quick screens, 6–7 November 2026

Raw results of the paired screens run on hana with `quick.py` and `mw.py` between 6 and 7 November 2026, before the timer benchmark moved to `base-benchmark`. The [performance investigations](../../../../performance/index.md) link the files they rely on. They were copied from `~/Developer/claude-bench/quick/` on hana on 8 November 2026. The source trees and build directories there are not part of this repository; [artifacts.txt](artifacts.txt) records the executables they held then.

Host: hana, Intel Xeon E3-1240 v6 (4 cores, 8 threads), Linux 7.2, Rust 1.99.1, release builds, `powersave` governor. Every build and run held `/tmp/base-scheduler-perf.lock`.

| Path | Contents |
| --- | --- |
| [quick.py](quick.py) | The driver: `quick.py echo`, `http`, or `timers`, with BASE and CANDIDATE source trees. It builds both, runs each case with the builds in a random order within each round, and prints the median of per-round changes with the rounds that improved. Unchanged controls (raw Tokio, Hyper) run in the same rounds. Its own description of the layout control is wrong for echo; see below. |
| [harness/](harness) | The timer diagnostic `quick.py timers` builds against each tree, from [rescue-costs](../../../timers/results/rescue-costs/readme.md). |
| [http-source.diff](http-source.diff) | `quick.py http`'s hello benchmark: base-http `v0.3.0` with the Direct backend removed and the scheduler pinned to 0.4. |
| [mw/](mw) | `mw.py`, the multi-worker HTTP driver, with its `[patch]` files, diagnostic and profiling scripts, results, and [http-source.diff](mw/http-source.diff): base-http `v0.3.0`'s hello benchmark with a mode that spawns one task per connection. |
| [results/](results) | One JSON line per sample, named by mode, topic, and start time. An empty file from an aborted start is omitted, and files already committed elsewhere are listed below rather than copied. |
| `runnable-counter-{1,2,3}.diff` | The source of the three rejected runnable-counter candidates, against their base `140cc25`, recovered from the screened trees. |
| [artifacts.txt](artifacts.txt) | SHA-256 prefixes of the executables in the build directories on hana on 8 November. |
| `selftest-*.txt`, `pr105-rerun.txt` | Printed summaries of the driver's self-tests and of a rerun for #105. |

Results of these screens committed before this directory existed:

| File on hana | Committed copy |
| --- | --- |
| `echo-pr105-20261106-223038.jsonl` | [driver-wakes/quick-4w-3.jsonl](../driver-wakes/quick-4w-3.jsonl) |
| `echo-work-distribution-1-20261106-{213752,215603,221038}.jsonl` | [driver-wakes/quick-4w-1, quick-4w-2, quick-1w](../driver-wakes) |
| `echo-unified-receive-4-20261107-105059.jsonl`, `-5-…-112016`, `-6-…-124018`, `-6-…-130935` | [unified-receive/quick-1 to quick-4](../unified-receive), in that order; the second `-6` file is run 4 |
| `echo-timer-batching-1-20261106-220354.jsonl`, `timers-main-20261106-200604.jsonl`, `timers-timer-batching-1-20261106-{215125,222817}.jsonl` | [timer-batching/quick](../../../timers/results/timer-batching/quick) |

## Controls

`quick.py` gained its default "layout" control partway through 6 November: a build of BASE from a copy at another path, run in the same rounds, on the assumption that Cargo hashes each package's path into its build. Which control a run had:

| Runs | Control | What it measured |
| --- | --- | --- |
| `*-main-copy-*` (14:03–14:09) | Hand-made A/A: a copy of `main` as the candidate | For echo, the same executable; for `http` and `timers`, a different build. |
| `echo-owned-readiness-1/2`, `echo-runnable-counter-1/2`, `echo-main-20261106-204901`, `timers-main-*`, every `http-*` file, `iouring-regression-multi-1` | None from `quick.py` | Run-to-run noise is visible only through the unchanged Tokio or Hyper control. |
| Later `echo-*` runs (with a `copy` build) | `quick.py`'s copy | The same executable as BASE. |
| `timers-timer-batching-*`, `mw-*` | `quick.py`'s or `mw.py`'s copy | A different build: a genuine layout sample. |

**Echo copies were the same executable.** The echo benchmark is a member of the tree's own workspace, and Cargo identifies workspace members by their paths relative to the workspace root, so the copy built byte-identically. For example, `main` and `main-layout` were both `30ed5e22…`, and `main-cd51eee` and its copy both `4560cae1…` ([artifacts.txt](artifacts.txt)). An echo "layout" change therefore measures run-to-run variation of one executable, not build layout; build layout was not controlled in any echo screen. The "fresh tree" confirmation runs were the same executable as their first runs too, for example `owned-vectored-1` and `-2`.

**Out-of-tree harnesses gave genuine layout samples.** The timer diagnostic, `quick.py http`'s hello benchmark, and `mw.py` depend on the tree by absolute path, so a copy's path changes their crate hashes: the timer diagnostic's base, candidate, and copy were `79fe9246…`, `103584f0…`, and `bdbded8d…`, and `mw.py`'s `88ec9207…`, `8af1fcf3…`, and `6d51c5c2…`. No `http-*` run had a `quick.py` copy; the HTTP layout comparisons used hand-made copies, such as `iouring-regression-aa`, a copy of `v0.4.0`.

The investigations state which applies. Comparisons of build layout should now use `base-benchmark`'s layout replicas, which are salted so that they differ, and warn when one does not.
