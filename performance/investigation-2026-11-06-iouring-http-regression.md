# Performance Investigation: An apparent io_uring HTTP regression since v0.4.0

## Identity and status

- Investigation ID: `performance/investigation-2026-11-06-iouring-http-regression`.
- Date and author/reviewer: 6 November 2026; Claude for Samuel Williams; not reviewed.
- Evidence class: screening.
- Evidence validity: inconclusive. The committed runs show no consistent regression, and differences of the same size between two builds of identical source. The working log's figure of −1.7% to −3.8% cannot be traced to a retained file.
- Decision: rejected (nothing to change).
- Related investigations: none.

## Hypothesis and constraints

The working log recorded io_uring HTTP hello as 1.7–3.8% slower on `main` than on `v0.4.0`. Stall rescue, merged in between, was the suspect.

## Frozen variants and preparation

`quick.py http` builds a private copy of base-http's hello benchmark ([http-source.diff](../benchmarks/echo/results/quick-screens/http-source.diff)) against each tree by absolute path. Trees on hana: `v0.4.0`, `main` of 6 November, `iouring-regression-aa` (a copy of `v0.4.0` at another path), and `iouring-regression-1` to `-3`. Their last builds are in [artifacts.txt](../benchmarks/echo/results/quick-screens/artifacts.txt); `v0.4.0` (`485ef9e7…`) and its copy (`b4864dea…`) were different builds of the same source. Which tree was BASE and which CANDIDATE in each `http-*` run was not recorded. The labels `r2`, `r3`, and `r7b` in the multi-tree file name revisions that were not recorded, and the script that wrote that file was not kept.

## Predeclared method

`quick.py http` on hana: io_uring and readiness backends, 16, 64, and 256 connections, Hyper as the unchanged control, interleaved rounds, with no copy control ([controls](../benchmarks/echo/results/quick-screens/readme.md#controls)). `perf stat` compared one run of each build.

## Evidence and results

- Raw bundles, in [quick-screens/results](../benchmarks/echo/results/quick-screens/results): `http-main-copy-20261106-140540` (an early A/A run of `main` against a copy), `http-main-20261106-194605`, `http-iouring-regression-1-20261106-194627`, `http-iouring-regression-aa-20261106-203252`, and `iouring-regression-multi-1`. Changes are medians of per-round changes in requests/s for io_uring, with rounds improved.

| Run | 16 connections | 64 connections | 256 connections |
| --- | --- | --- | --- |
| `http-iouring-regression-1` | −3.4% (0/5) | | +0.8% (3/5) |
| `http-iouring-regression-aa`, presumably `v0.4.0` against its copy | −0.5% (2/5) | −1.1% (1/5) | −0.6% (2/5) |
| Multi-tree, `main` against `v0.4.0` | −0.1% (2/5) | | +0.0% (3/5) |
| Multi-tree, the copy against `v0.4.0` | −3.5% (2/5) | | −2.3% (0/5) |
| Multi-tree, `r2`, `r3`, `r7b` against `v0.4.0` | −1.5% to +1.9% | | −2.4% to −0.1% |
| `http-main-copy` A/A, 2 rounds | +0.8% | −5.3% | −0.3% |

`perf stat` showed the same instructions and cycles per request for `main` and `v0.4.0`; its output was not kept. The server registers no timers, so stall rescue's monitor never runs there.

## Decision and code disposition

No regression was established: in the multi-tree run `main` matched `v0.4.0`, while `v0.4.0`'s own copy moved by 2–4%. Nothing was changed.

## Revisit conditions

A difference beyond salted layout replicas in a `base-benchmark` campaign, or a change in instructions per request.
