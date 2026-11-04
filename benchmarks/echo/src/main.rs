// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    base_scheduler_benchmark_echo::run(&std::env::args().collect::<Vec<_>>())
}
