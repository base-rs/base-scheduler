# Validation record

The selected production behavior is commit `9e005c5061c26bed937953d1e540a3ddde5cc6ed`, based on merged PR #14 (`a98f172f8b06f23b34a354a4b6f6c294aa5859ab`). The frozen benchmark executable uses that exact tree; `build/source.patch`, `build/tree.txt`, and `provenance.json` identify it independently of the later documentation.

Linux and macOS passed formatting, workspace Clippy for all targets and features with warnings denied, API documentation with warnings denied, and the canonical all-targets/all-features Bake source-region coverage gate. Linux measured 18,491/18,491 source regions, with one invariant-only `unreachable!()` region excluded by the shared task. macOS measured 15,891/15,891, with no exclusions. The [Linux logs](logs/) preserve each command's output; [macOS coverage output](logs/macos-coverage.log) preserves that gate. Linux uses its installed LLVM tools with a wrapper excluding only the Arch-installed compiler's own sources, not project code.

Independent adversarial review of the selected change found no substantive correctness issues. It requested explicit initial blocking-publication race coverage and removal of a stale comment. Commit `1547fa6c7f915637b215acc87de7174e68e160db` adds those test cases for both standalone drivers and entered workers; it changes no production behavior. Tests also cover already-published notification polls, notifications before first nonblocking publication, local permits, CQ-driven counter draining, interrupted reads, one-entry submission queues, shutdown, and cancellation.

The follow-up passed the complete `core-scheduler-iouring` package test run on Linux, including conformance and documentation tests; [review-tests.log](logs/review-tests.log) records it. The benchmark executable and coverage numbers above remain explicitly attributed to the selected production commit rather than this test-only follow-up.

The benchmark holds the exclusive `/tmp/core-scheduler-perf.lock` for all timed samples. Remote builds and tests acquire its shared mode and do not overlap timings. Separate experimental worktrees leave the user's canonical Aiko checkout untouched.

Windows does not compile the changed Linux-only implementation. Its workspace checks remain part of the pull request CI matrix. No public capability, ownership, cancellation, resource-affinity, or shutdown contract changes.
