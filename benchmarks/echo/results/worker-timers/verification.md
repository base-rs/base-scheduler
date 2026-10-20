# Prototype verification

The candidate patch is diagnostic evidence and is not applied to the workspace. Both optimized builds compile the same timer harness. Source and hashes are in `build.json`.

- macOS: 57 Native unit tests passed, including the two added driver/timer tests.
- Linux: the same 57 Native unit tests passed (`cargo test --locked -p core-scheduler-native --lib`).
- macOS: the shared Bake coverage task ran the package’s documentation, unit and integration tests successfully. Coverage was **1,467/1,477 measured regions**, leaving ten regions uncovered in the experimental integration. This does **not** satisfy the production coverage gate.

The added tests check that an earlier external deadline interrupts an existing driver wait without starting a timer thread, dropping the distant timer does not produce an idle spin, and shutdown waits for timer wake delivery outside the queue lock. The timer workload also asserts that no timer returns before its requested deadline.

The first local coverage invocation used Homebrew Rust without matching LLVM tools and could not merge profiles. Repeating with `rustup run stable cargo bake --locked test:coverage --package core-scheduler-native --all-targets true --all-features true` produced the measured-region result above. Full logs remain in the local research cache.

Before any production proposal, add deterministic registration/parking race tests, integrated cancellation/capacity/shutdown coverage, timer progress after driver retirement, and multiworker coverage; then pass the full supported-platform coverage workflow. Also validate constructor arguments before reserving timer storage, and simplify the candidate’s unused requested-timeout parameter: its absent-deadline behavior is unsuitable for a caller-supplied timeout.

After archiving the candidate and restoring production source, the same shared Bake package coverage task passed on macOS: **1,335/1,335 measured regions**. The retained change consists of research evidence and documentation.
