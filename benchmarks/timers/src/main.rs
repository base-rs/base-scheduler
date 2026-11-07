// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

fn main() -> std::process::ExitCode {
    base_scheduler_benchmark_timers::command(
        std::env::args().skip(1),
        &mut std::io::stdout(),
        &mut std::io::stderr(),
    )
}
