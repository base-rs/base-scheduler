// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

fn main() {
    if let Err(error) = core_scheduler_write_benchmark::run(&std::env::args().collect::<Vec<_>>()) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
