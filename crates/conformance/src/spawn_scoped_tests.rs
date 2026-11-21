// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

/// Generate separately reported scoped spawning tests. The runner supplies a fresh `SpawnScopedFixture` reference per case.
///
/// Invoke inside a dedicated module. Each case is synchronous, because `SpawnScoped::scope` blocks: the runner calls it once on a thread outside the scheduler's workers, enforces a deadline, shuts the scheduler down, and propagates panics.
#[macro_export]
macro_rules! spawn_scoped_tests {
    (run: $run:path $(,)?) => {
        $crate::__conformance_blocking_test_cases! { $run;
            borrowed_output => $crate::spawn_scoped::borrowed_output,
            parallel => $crate::spawn_scoped::parallel,
            pending_abandonment => $crate::spawn_scoped::pending_abandonment,
            completed_abandonment => $crate::spawn_scoped::completed_abandonment,
            nested => $crate::spawn_scoped::nested,
            root_panic => $crate::spawn_scoped::root_panic,
            task_panic => $crate::spawn_scoped::task_panic,
            closed => $crate::spawn_scoped::closed,
        }
    };
}
