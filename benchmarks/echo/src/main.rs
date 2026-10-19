// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    core_scheduler_echo_benchmark::run(&std::env::args().collect::<Vec<_>>())
}
