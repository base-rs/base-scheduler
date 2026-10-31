// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

/// Generate adopted pipe cases for a scheduler implementing `PipeReading` and `PipeWriting`. The runner supplies a fresh scheduler reference and retires it after each async case. Cases run serially so that a concurrent child spawn cannot retain another case's pipe end. Unix only.
#[macro_export]
macro_rules! pipe_tests {
    (run: $run:path $(,)?) => {
        #[test]
        fn pipe_child_round_trip() {
            let _serial = $crate::pipe::serial();
            $run(async |scheduler| {
                $crate::pipe::child_round_trip(scheduler).await;
            });
        }
        #[test]
        fn pipe_vectored() {
            let _serial = $crate::pipe::serial();
            $run(async |scheduler| {
                $crate::pipe::vectored(scheduler).await;
            });
        }
        #[test]
        fn pipe_end_of_file() {
            let _serial = $crate::pipe::serial();
            $run(async |scheduler| {
                $crate::pipe::end_of_file(scheduler).await;
            });
        }
        #[test]
        fn pipe_empty_transfers() {
            let _serial = $crate::pipe::serial();
            $run(async |scheduler| {
                $crate::pipe::empty_transfers(scheduler).await;
            });
        }
        #[test]
        fn pipe_broken_pipe() {
            let _serial = $crate::pipe::serial();
            $run(async |scheduler| {
                $crate::pipe::broken_pipe(scheduler).await;
            });
        }
        #[test]
        fn pipe_cancellation() {
            let _serial = $crate::pipe::serial();
            $run(async |scheduler| {
                $crate::pipe::cancellation(scheduler).await;
            });
        }
        #[test]
        fn pipe_close() {
            let _serial = $crate::pipe::serial();
            $run(async |scheduler| {
                $crate::pipe::close(scheduler).await;
            });
        }
    };
}
