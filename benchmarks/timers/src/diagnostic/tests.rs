// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;

fn diagnostic(values: &[&str]) -> serde_json::Value {
    execute(
        &values
            .iter()
            .map(|value| value.to_string())
            .collect::<Vec<_>>(),
    )
}

#[test]
fn backends_and_alignments_record_expiries_inside_the_window() {
    #[cfg(target_os = "linux")]
    let backends = ["native", "iouring"];
    #[cfg(not(target_os = "linux"))]
    let backends = ["native"];
    for backend in backends {
        for alignment in ["unaligned", "aligned"] {
            let record = diagnostic(&[backend, "2", "4", "1000", alignment, "30", "10"]);
            assert_eq!(record["backend"], backend);
            assert_eq!(record["alignment"], alignment);
            assert_eq!(record["work"], 10);
            assert!(record["completed"].as_u64().unwrap() > 0, "{record}");
            assert!(record["lateness_p50_us"].is_number());
            assert_eq!(record["rescues"].is_object(), cfg!(feature = "diagnostics"));
        }
    }
}

#[test]
fn a_window_shorter_than_the_delay_records_no_expiry() {
    let record = diagnostic(&["native", "1", "1", "5000", "unaligned", "1"]);
    assert_eq!(record["completed"], 0);
    assert!(record["lateness_p50_us"].is_null());
    assert_eq!(record["work"], 0);
}

#[test]
#[should_panic(expected = "unknown alignment")]
fn an_unknown_alignment_is_rejected() {
    diagnostic(&["native", "1", "1", "1000", "sideways"]);
}

#[test]
#[should_panic(expected = "unknown backend")]
fn an_unknown_backend_is_rejected() {
    diagnostic(&["unknown", "1", "1", "1000", "unaligned", "1"]);
}
