// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;

fn parse(values: &[&str]) -> io::Result<Benchmark> {
    Benchmark::parse(values.iter().map(|value| value.to_string()))
}

fn error(values: &[&str]) -> String {
    parse(values).unwrap_err().to_string()
}

#[test]
fn every_name_parses_to_its_variant() {
    fn round_trip<T: FromStr<Err = io::Error> + Copy + PartialEq + std::fmt::Debug>(
        all: &[T],
        name: fn(T) -> &'static str,
        what: &str,
    ) {
        for &value in all {
            assert_eq!(name(value).parse::<T>().unwrap(), value);
        }
        let error = "sideways".parse::<T>().unwrap_err().to_string();
        assert_eq!(error, format!("unknown {what} \"sideways\""));
    }
    round_trip(Backend::ALL, Backend::name, "backend");
    round_trip(NativeBackend::ALL, NativeBackend::name, "Native backend");
    round_trip(Mode::ALL, Mode::name, "mode");
    round_trip(TimerWorkload::ALL, TimerWorkload::name, "workload");
    round_trip(ScenarioWorkload::ALL, ScenarioWorkload::name, "scenario");
    round_trip(Alignment::ALL, Alignment::name, "alignment");
    assert_eq!(
        Backend::ALL.len(),
        2 + usize::from(cfg!(target_os = "linux"))
    );
}

#[test]
fn each_family_has_defaults_and_records_every_parameter() {
    let features = json!(FEATURES);
    let timer = parse(&[]).unwrap();
    assert_eq!(
        timer,
        Benchmark::Timer(TimerOptions {
            workload: TimerWorkload::Refresh,
            backend: Backend::Native,
            mode: Mode::After,
            tasks: 1,
            iterations: 100_000,
            workers: 1,
            delay: Duration::from_millis(1),
        })
    );
    assert_eq!(
        timer.parameters(),
        json!({"workload": "refresh", "backend": "native", "mode": "after", "tasks": 1, "iterations": 100_000, "workers": 1, "delay_us": 1000, "features": features})
    );

    let scenario = parse(&["--workload", "task-yield"]).unwrap();
    assert_eq!(
        scenario,
        Benchmark::Scenario(ScenarioOptions {
            workload: ScenarioWorkload::TaskYield,
            workers: 1,
            iterations: 100,
            unit: Duration::from_millis(1),
        })
    );
    assert_eq!(
        scenario.parameters(),
        json!({"workload": "task-yield", "backend": "native", "workers": 1, "iterations": 100, "unit_us": 1000, "features": features})
    );

    let periodic = parse(&["--workload", "periodic"]).unwrap();
    assert_eq!(
        periodic,
        Benchmark::Periodic(PeriodicOptions {
            backend: NativeBackend::Native,
            workers: 4,
            tasks: 1024,
            delay: Duration::from_millis(1),
            alignment: Alignment::Unaligned,
            measure: Duration::from_secs(3),
            work: 0,
        })
    );
    assert_eq!(
        periodic.parameters(),
        json!({"workload": "periodic", "backend": "native", "workers": 4, "tasks": 1024, "delay_us": 1000, "alignment": "unaligned", "measure_ms": 3000, "work": 0, "features": features})
    );
}

#[test]
fn every_option_of_each_family_is_accepted() {
    let timer = parse(&[
        "--workload",
        "cycle",
        "--backend",
        "tokio",
        "--mode",
        "sleep",
        "--tasks",
        "3",
        "--iterations",
        "4",
        "--workers",
        "2",
        "--delay-us",
        "0",
    ])
    .unwrap();
    assert_eq!(timer.parameters()["delay_us"], 0);
    assert_eq!(timer.parameters()["mode"], "sleep");
    let scenario = parse(&[
        "--workload",
        "stall",
        "--workers",
        "2",
        "--iterations",
        "3",
        "--unit-us",
        "10",
    ])
    .unwrap();
    assert_eq!(scenario.parameters()["unit_us"], 10);
    let periodic = parse(&[
        "--workload",
        "periodic",
        "--workers",
        "2",
        "--tasks",
        "8",
        "--delay-us",
        "500",
        "--alignment",
        "aligned",
        "--measure-ms",
        "20",
        "--work",
        "5",
    ])
    .unwrap();
    assert_eq!(periodic.parameters()["alignment"], "aligned");
    assert_eq!(periodic.parameters()["work"], 5);
}

#[test]
fn malformed_and_inapplicable_options_are_errors() {
    assert_eq!(
        error(&["refresh"]),
        "expected --name value, found \"refresh\""
    );
    assert_eq!(error(&["--bogus", "1"]), "unknown option --bogus");
    assert_eq!(error(&["--tasks"]), "missing value for --tasks");
    assert_eq!(
        error(&["--tasks", "1", "--tasks", "2"]),
        "duplicate option --tasks"
    );
    assert_eq!(
        error(&["--tasks", "many"]),
        "invalid --tasks \"many\": invalid digit found in string"
    );
    assert_eq!(
        error(&["--iterations", "0"]),
        "--iterations must be positive"
    );
    assert_eq!(
        error(&["--workload", "sideways"]),
        "unknown workload \"sideways\""
    );
    assert_eq!(
        error(&["--unit-us", "10"]),
        "--unit-us does not apply to the refresh workload"
    );
    assert_eq!(
        error(&["--workload", "spawn", "--mode", "reuse"]),
        "--mode does not apply to the spawn workload"
    );
    assert_eq!(
        error(&["--workload", "spawn", "--backend", "native"]),
        "--backend does not apply to the spawn workload"
    );
    assert_eq!(
        error(&["--workload", "periodic", "--iterations", "1"]),
        "--iterations does not apply to the periodic workload"
    );
    assert_eq!(
        error(&["--workload", "periodic", "--backend", "tokio"]),
        "invalid --backend \"tokio\": unknown Native backend \"tokio\""
    );
    assert_eq!(
        error(&["--workload", "periodic", "--delay-us", "0"]),
        "--delay-us must be positive"
    );
    assert_eq!(
        error(&["--workload", "spawn", "--unit-us", "0"]),
        "--unit-us must be positive"
    );
}

#[test]
fn every_option_rejects_an_invalid_value() {
    let families: [&[&str]; 3] = [
        &[
            "--backend",
            "--mode",
            "--tasks",
            "--iterations",
            "--workers",
            "--delay-us",
        ],
        &["--workers", "--iterations", "--unit-us"],
        &[
            "--backend",
            "--workers",
            "--tasks",
            "--delay-us",
            "--alignment",
            "--measure-ms",
            "--work",
        ],
    ];
    for (workload, options) in ["refresh", "spawn", "periodic"].into_iter().zip(families) {
        for option in options {
            let message = error(&["--workload", workload, option, "-"]);
            assert!(
                message.starts_with(&format!("invalid {option} \"-\": ")),
                "{workload} {option}: {message}"
            );
        }
    }
}

#[cfg(feature = "allocations")]
#[test]
fn the_allocation_workload_is_available_with_its_feature() {
    let benchmark = parse(&["--workload", "alloc"]).unwrap();
    assert_eq!(benchmark.parameters()["workload"], "alloc");
    assert!(FEATURES.contains(&"allocations"));
}

#[cfg(not(feature = "allocations"))]
#[test]
fn the_allocation_workload_needs_its_feature() {
    assert!(error(&["--workload", "alloc"]).contains("needs the allocations feature"));
    assert!(!FEATURES.contains(&"allocations"));
}
