# Performance Investigation: Multi-worker HTTP and io_uring poll rebinding

## Identity and status

- Investigation ID: `performance/investigation-2026-11-07-multi-worker-http`.
- Date and author/reviewer: 6–7 November 2026; Claude for Samuel Williams; the change was reviewed and merged in #109.
- Evidence class: screening.
- Evidence validity: valid for the io_uring change against a genuine layout replicate; the comparison with Hyper is at about the same load, not capacity, because wrk limits throughput.
- Decision: adopted (keep an active poll on its ring, #109); open (Native's p99 at 4 workers, and the questions below).
- Related investigations: [driver wakes](investigation-2026-11-06-driver-wakes.md), whose driver-local rule returns a woken task to its ring's worker.

## Hypothesis and constraints

This was the first measurement of multi-worker HTTP against Hyper on a multi-thread Tokio runtime. Run 1, against `main` at `3738ff6`, found Native epoll ahead of Hyper at every load (+6% to +17% throughput, −7% to −15% CPU per request), but io_uring behind Hyper in four of six cases, by 2.5–9.9%. Profiles put io_uring's extra work in monitor handling, cross-ring commands, and `route_ref`. When an idle worker stole a connection task and it waited again, the persistent multishot `PollAdd` was cancelled on the old ring and registered on the new one. A diagnostic build counted 0.30–0.49 rebinds per request at 2–4 workers. Keeping an active poll on its ring should remove that work.

No wakeup may be lost: the wait and `notify` meet under the status lock, so which ring delivers the notification does not matter. Close, drop, and shutdown must still retire through the poll's own ring.

## Frozen variants and preparation

`3738ff6` against `3738ff6` plus the change, each with a private copy of base-http's hello benchmark that spawns one task per connection (`base_http_server::serve` polls every connection in one task), committed as [http-source.diff](../benchmarks/echo/results/quick-screens/mw/http-source.diff) against base-http `v0.3.0`. `mw.py` builds it against each tree by absolute path, so its copy build is a genuine layout replicate ([controls](../benchmarks/echo/results/quick-screens/readme.md#controls)). The `[patch]` files are in [mw/patches](../benchmarks/echo/results/quick-screens/mw/patches).

## Predeclared method

`mw.py` on hana: 2 and 4 workers, 16, 64, and 256 connections, wrk on the remaining CPUs (at 4 workers, the server's SMT siblings), Hyper as the unchanged control, 5 interleaved rounds per run, two runs (the second from a fresh tree with seed 7). An echo cross-check at 4 workers used `quick.py echo`.

## Evidence and results

- Raw bundles: [mw-run1](../benchmarks/echo/results/quick-screens/mw/results/mw-run1.jsonl), [mw-keep-binding-1 and -2](../benchmarks/echo/results/quick-screens/mw/results), and [echo-multiworker-http-2](../benchmarks/echo/results/quick-screens/results/echo-multiworker-http-2-20261107-020038.jsonl). #109's description has the full tables.

| Metric and scope | Change, rounds improved | Layout replicate | Validity and interpretation |
| --- | --- | --- | --- |
| io_uring requests/s, 2 and 4 workers | +14% to +26%, 5/5 in every case, two runs | within ±2.1% | Valid. |
| io_uring CPU/request | −15% to −24%, 5/5 | | Valid. |
| io_uring against Hyper | run 1 of the change: from −6.6% to +4.0% before to +8.6% to +20.7% after; run 2: from −11.3% to +3.3% to +4.0% to +20.0% | | At about the same load, since wrk limits throughput; level with epoll. |
| Echo, borrowed io_uring, 4 workers, CPU/request | −7% to −12%, 5/5 | same executable | Valid against run-to-run noise. |
| Native epoll | unchanged | up to +2.5% | As expected. |

## Decision and code disposition

Adopted in #109, with tests that a later wait on another worker keeps the active poll on its ring and that a new poll uses the current worker's ring. The multi-worker benchmark mode is not part of base-http; its source is the diff above.

## Revisit conditions

These remain open:
- Native's p99 at 4 workers is worse than Hyper's at 16 and 64 connections, about 870–1,540 µs against 433–880. Not investigated.
- Owned io_uring operations still choose the current worker's ring per operation; their cost under migration is untested.
- A socket's reader and writer monitors can bind to different rings, each by its own first wait, so a connection with backpressure in both directions could move between two workers on each event. The hello benchmark rarely waits for writability, so this is not measured.
- base-http may want a `serve` variant that spawns a task per connection, which would make this benchmark reproducible from its repository.
