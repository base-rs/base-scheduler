// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

#[cfg(target_os = "linux")]
#[path = "pipeline/linux.rs"]
mod linux;

fn main() {
    #[cfg(target_os = "linux")]
    println!("{}", linux::execute(&std::env::args().collect::<Vec<_>>()));
    #[cfg(not(target_os = "linux"))]
    panic!("The zero-copy pipeline experiment requires Linux");
}

#[cfg(all(test, not(target_os = "linux")))]
#[test]
#[should_panic(expected = "The zero-copy pipeline experiment requires Linux")]
fn unsupported_platform_reports_its_requirement() {
    main();
}
