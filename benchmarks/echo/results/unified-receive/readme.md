# One receive rule for every stream receive

Every stream receive which consumes input now follows one receive rule on each backend: once a reader has received data it waits on cached readiness before its syscall, a short receive clears readiness when OOB support is disabled, and Linux epoll and io_uring's readiness-driven receives reject reported urgent data. Before this change, `read_vectored`, `read_vectored_with_flags`, `read_into_with_flags`, `read_owned_vectored` and pooled `read` made a syscall first on every call and kept readiness until `WouldBlock`, so an echo request cost one extra receive returning `WouldBlock`. Peeks keep that conservative behavior. See the [urgent-data policy](../../../../design/socket-stream-oob.md).

## Comparison

`quick.py echo` on Hana (Xeon E3-1240 v6) compares `main` (`00cadff`) plus only the commit adding the io_uring and Tokio adapter vectored, owned vectored and pooled echo backends, with this change. The server runs on CPUs 0 to workers-1 and the client on CPUs 4–7, with 32 connections and 64-byte messages. Raw Tokio and a layout control, the base source built at another path, run in the same rounds. Values are medians of five rounds; "better" counts rounds that improved on the base. `iouring-owned-vectored` is not listed: io_uring's owned vectored reads submit `RecvMsg` and are unchanged.

[Run 1](quick-1.jsonl), and [run 2](quick-2.jsonl) with `--seed 7` and a fresh tree copy. Requests per second and CPU µs per request, change from base (rounds better), with the layout control's change:

| Case | Backend | Run 1 req/s | Run 1 CPU/req | Run 2 req/s | Run 2 CPU/req | Layout req/s (run 1, run 2) |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| 1w | native-vectored | +10.6% (5/5) | −14.1% (5/5) | +15.2% (5/5) | −17.4% (5/5) | −2.2%, +2.5% |
| 1w | native-owned-vectored | +16.2% (5/5) | −17.5% (5/5) | +15.4% (5/5) | −17.3% (5/5) | +1.9%, +1.3% |
| 1w | native-pooled | +17.9% (5/5) | −19.4% (5/5) | +13.8% (5/5) | −16.5% (5/5) | +1.6%, +0.9% |
| 1w | iouring-vectored | +13.5% (5/5) | −15.7% (5/5) | +13.2% (5/5) | −16.4% (5/5) | +1.2%, +1.2% |
| 1w | iouring-pooled | +15.4% (5/5) | −17.5% (5/5) | +15.8% (5/5) | −17.7% (5/5) | −0.3%, −0.5% |
| 1w | tokio-adapter-vectored | +35.4% (5/5) | −33.0% (5/5) | +33.8% (5/5) | −32.3% (5/5) | +3.7%, +6.1% |
| 1w | tokio-adapter-owned-vectored | +36.2% (5/5) | −33.0% (5/5) | +35.8% (5/5) | −32.8% (5/5) | −0.4%, +0.4% |
| 1w | tokio-adapter-pooled | +16.5% (5/5) | −17.4% (5/5) | +17.3% (5/5) | −18.4% (5/5) | +0.0%, −0.7% |
| 4w | native-vectored | +1.8% (5/5) | −6.4% (5/5) | +3.7% (5/5) | −7.3% (5/5) | −1.8%, +0.3% |
| 4w | native-owned-vectored | +2.9% (5/5) | −6.2% (5/5) | +2.1% (5/5) | −5.8% (5/5) | +1.1%, +0.6% |
| 4w | native-pooled | +2.1% (4/5) | −7.4% (5/5) | +2.3% (4/5) | −5.8% (5/5) | −0.5%, −0.6% |
| 4w | iouring-vectored | +3.3% (5/5) | −6.8% (5/5) | +2.6% (5/5) | −6.9% (5/5) | +1.0%, −1.2% |
| 4w | iouring-pooled | +4.6% (5/5) | −6.7% (5/5) | +3.1% (3/5) | −6.9% (5/5) | −0.1%, −1.0% |
| 4w | tokio-adapter-vectored | +13.9% (5/5) | −19.6% (5/5) | +13.1% (5/5) | −20.3% (5/5) | −0.3%, −0.8% |
| 4w | tokio-adapter-owned-vectored | +15.9% (5/5) | −21.2% (5/5) | +13.9% (5/5) | −19.6% (5/5) | +0.3%, −1.3% |
| 4w | tokio-adapter-pooled | +5.4% (5/5) | −11.0% (5/5) | +7.2% (5/5) | −11.0% (5/5) | +1.0%, −0.0% |

The scalar backends, whose receive rule was already the target, stay within the layout control:

| Case | Backend | Run 1 req/s | Run 1 CPU/req | Run 2 req/s | Run 2 CPU/req | Layout CPU/req (run 1, run 2) |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| 1w | native | +0.7% (3/5) | −0.6% (3/5) | −0.1% (2/5) | −0.1% (3/5) | −0.3%, −0.0% |
| 1w | native-poll | −0.1% (2/5) | +0.3% (2/5) | +0.2% (3/5) | −0.2% (3/5) | +1.4%, −1.3% |
| 1w | iouring | +0.3% (4/5) | −0.1% (4/5) | +0.3% (3/5) | −0.4% (3/5) | +0.0%, +1.2% |
| 1w | iouring-poll | +0.1% (3/5) | −0.2% (4/5) | +1.2% (4/5) | −1.3% (4/5) | +0.3%, −1.7% |
| 1w | tokio-adapter | −1.2% (1/5) | +2.0% (1/5) | −1.5% (0/5) | +2.2% (0/5) | +0.8%, +2.2% |
| 1w | tokio-adapter-poll | +0.5% (3/5) | +0.1% (2/5) | +1.3% (3/5) | −1.5% (3/5) | −2.3%, +0.5% |
| 4w | native | +0.2% (3/5) | −0.8% (3/5) | +1.1% (5/5) | −1.2% (5/5) | −0.4%, +0.4% |
| 4w | native-poll | −0.8% (2/5) | +0.2% (2/5) | −1.6% (2/5) | −1.1% (4/5) | −0.4%, −0.5% |
| 4w | iouring | −1.0% (1/5) | +1.1% (1/5) | −0.1% (2/5) | −0.5% (3/5) | +0.1%, −0.5% |
| 4w | iouring-poll | +1.5% (4/5) | −1.2% (3/5) | −0.3% (2/5) | +0.4% (2/5) | +0.7%, +0.6% |
| 4w | tokio-adapter | +3.3% (4/5) | −1.8% (3/5) | −1.2% (2/5) | −0.0% (3/5) | +0.2%, −0.2% |
| 4w | tokio-adapter-poll | +1.1% (3/5) | +1.5% (2/5) | −2.7% (2/5) | −0.1% (4/5) | +0.3%, +1.8% |

`tokio-adapter` with one worker used about 2% more CPU per request in both runs, within the layout control in run 2 but not in run 1. Counting its server's user-space work with `perf stat` on Hana (five interleaved samples each) found the same instructions per request as the base (2,102 against 2,096; the layout copy 2,085) and user cycles within the layout copy's (4,412 against 4,440; base 4,245), so the remaining difference is build layout rather than added work.

## Review revision

The review revision waits before taking a slot in the Tokio adapter's pooled reads too, lets selector peeks wait on the reader's registration, records EOF the same way on every backend, and expands one macro for each Tokio adapter read. [Run 3](quick-3.jsonl) repeats the comparison against the same base:

| Case | Backend | req/s | CPU/req | Layout req/s, CPU/req |
| --- | --- | ---: | ---: | ---: |
| 1w | native-vectored / owned-vectored / pooled | +13.6% / +14.8% / +14.2% (5/5) | −16.2% / −16.3% / −16.5% (5/5) | within ±1.8% |
| 1w | iouring-vectored / pooled | +11.1% / +15.7% (5/5) | −14.2% / −17.9% (5/5) | within ±1.1% |
| 1w | tokio-adapter-vectored / owned-vectored / pooled | +31.5% / +32.5% / +12.7% (5/5) | −30.8% / −30.7% / −15.2% (5/5) | +7.7%, −7.4% for vectored; else within ±2.4% |
| 4w | native-vectored / owned-vectored / pooled | +2.9% / +2.7% (5/5) / +3.3% (4/5) | −7.1% / −5.6% / −6.2% (5/5) | within ±1.8% |
| 4w | iouring-vectored / pooled | +3.1% / +3.8% (5/5) | −7.3% / −7.2% (5/5) | within ±1.0% |
| 4w | tokio-adapter-vectored / owned-vectored / pooled | +10.3% (5/5) / +14.6% / +3.5% (4/5) | −19.3% / −20.3% / −9.5% (5/5) | within ±2.2% |
| 1w | native / native-poll | +2.5% (5/5) / +0.1% (3/5) | −2.4% (5/5) / −0.4% (3/5) | within ±1.1% |
| 1w | iouring / iouring-poll | +0.9% (3/5) / −1.2% (2/5) | −0.4% (3/5) / +1.4% (2/5) | −0.7%, +0.9%; −1.3%, +1.3% |
| 1w | tokio-adapter / tokio-adapter-poll | −0.4% (2/5) / +0.1% (3/5) | −0.3% / −0.0% (3/5) | −1.3%, +1.1%; +0.1%, −0.1% |
| 4w | native / native-poll | −1.1% (2/5) / +0.1% (3/5) | −0.7% (5/5) / −0.5% (3/5) | within ±1.4% |
| 4w | iouring / iouring-poll | +1.0% (3/5) / +0.1% (3/5) | −0.9% (4/5) / +0.3% (1/5) | within ±2.0% |
| 4w | tokio-adapter / tokio-adapter-poll | −0.7% (2/5) / +2.0% (5/5) | +0.5% (2/5) / +0.3% (2/5) | within ±2.3% |

[Run 4](quick-4.jsonl) compares the revision with the previous revision directly. The Tokio adapter paths and native-poll changed by at most 2.4% in either direction, within the layout control except `tokio-adapter-owned-vectored` with one worker (−1.8% requests per second, +1.4% CPU per request, 0/5; layout −0.9%, +1.0%). `perf stat` on Hana found the same user instructions per request for it (3,434 against 3,440) with more user cycles (6,727 against 6,432; layout copy 6,473), so the difference is build layout. The pooled wait-first read executes 16% fewer user instructions per request (3,026 against 3,615) but saves only about 2% of user cycles and no measurable time; it is kept because it never measured slower and makes the pooled read follow the rule on every backend.

## Rejected variants

Instructions per echo request in the server, counted with `perf stat -e instructions:u` on a development machine, one worker, median of three to five interleaved samples. Each change is from the base measured in the same run:

| Variant | Backend | Variant's instructions | Adopted form's |
| --- | --- | ---: | ---: |
| Selector receive loop with a runtime flag distinguishing peeks | native-poll | 1,856 (+1.3%) | 1,839 (+0.0%) |
| Tokio adapter: one shared async helper awaiting either the established or the generic read | tokio-adapter-poll | 2,347 (+1.0%) | 2,327 (−0.0%) |
| The same helper | tokio-adapter | 2,494 (+3.2%) | 2,381 (+0.2%) |
| The same helper with `#[inline(always)]` | tokio-adapter | 2,431 (+0.6%) | |
| The same helper with its closure moved in, with and without `#[inline(always)]` | tokio-adapter | 2,399–2,400 (+0.9%) | |

The adopted forms keep peeks out of the shared receive loop, so it needs no flag: the readiness selector peeks through a small loop over the reader's cached registration, and io_uring and the Tokio adapter use their existing syscall-first operations. The Tokio adapter dispatches its established read through a labeled-block macro in each read rather than through a nested future. The Hana runs 1–3 of the first revision, with both rejected variants, showed the same gains on the converted paths and the scalar costs above.

## Correction, 8 November 2026

The "layout" control in these runs, the base built from a copy at another path, was the same executable as the base: the echo benchmark is a member of the tree's workspace, which Cargo identifies by relative path, so the copy built byte-identically (`e8894736…` for `unified-receive-base` and its copy). Its changes measure run-to-run variation of one executable, not build layout. The comparisons above hold against run-to-run noise. The `tokio-adapter` paragraph's conclusion that the remaining 2% "is build layout rather than added work" is therefore not supported as stated: instructions per request matched, so no work was added, but the difference in cycles is unexplained. See the [quick screens' controls](../quick-screens/readme.md#controls) and the [investigation record](../../../../performance/investigation-2026-11-06-stream-transfer-registration.md).
