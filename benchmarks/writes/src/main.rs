// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

fn main() -> std::process::ExitCode {
    if let Err(error) = base_scheduler_benchmark_writes::run(&std::env::args().collect::<Vec<_>>())
    {
        eprintln!("{error}");
        return std::process::ExitCode::FAILURE;
    }
    std::process::ExitCode::SUCCESS
}
