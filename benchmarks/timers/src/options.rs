// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The binary's options, written `--name value`. `--workload` selects one of three families of workloads, and each family accepts only its own options.

use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fmt::Display;
use std::io;
use std::str::FromStr;
use std::time::Duration;

// An enum whose variants each have a command-line name.
macro_rules! named {
    ($(#[$meta:meta])* pub enum $name:ident ($what:literal) { $($(#[$variant_meta:meta])* $variant:ident = $text:literal,)+ }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum $name {
            $($(#[$variant_meta])* $variant,)+
        }

        impl $name {
            /// Every variant, in declaration order.
            pub const ALL: &'static [Self] = &[$($(#[$variant_meta])* Self::$variant,)+];

            /// The command-line name.
            pub fn name(self) -> &'static str {
                match self {
                    $($(#[$variant_meta])* Self::$variant => $text,)+
                }
            }
        }

        impl FromStr for $name {
            type Err = io::Error;

            fn from_str(value: &str) -> io::Result<Self> {
                Self::ALL
                    .iter()
                    .copied()
                    .find(|candidate| candidate.name() == value)
                    .ok_or_else(|| invalid(format!(concat!("unknown ", $what, " {:?}"), value)))
            }
        }
    };
}

named! {
    /// A scheduler backend for the timer workloads.
    pub enum Backend ("backend") {
        Tokio = "tokio",
        Native = "native",
        #[cfg(target_os = "linux")]
        IoUring = "iouring",
    }
}

named! {
    /// A Native scheduler backend: its default thread-parking selector, or per-worker io_uring drivers.
    pub enum NativeBackend ("Native backend") {
        Native = "native",
        #[cfg(target_os = "linux")]
        IoUring = "iouring",
    }
}

named! {
    /// How a timer workload waits: a fresh sleep each time, a reused timer resource, or the reused resource behind the portable `After` wrapper.
    pub enum Mode ("mode") {
        Sleep = "sleep",
        Reuse = "reuse",
        After = "after",
    }
}

named! {
    /// A timer workload: refreshing, replacing and expiring timers from tasks.
    pub enum TimerWorkload ("workload") {
        Refresh = "refresh",
        Fixed = "fixed",
        Earlier = "earlier",
        EarlierYield = "earlier-yield",
        Yield = "yield",
        Expire = "expire",
        Cycle = "cycle",
        #[cfg(feature = "allocations")]
        Alloc = "alloc",
    }
}

named! {
    /// A Native scheduler scenario driven from the root thread.
    pub enum ScenarioWorkload ("scenario") {
        RootSleep = "root-sleep",
        Spawn = "spawn",
        SpawnPending = "spawn-pending",
        TaskSleep = "task-sleep",
        TaskSleepPending = "task-sleep-pending",
        Idle = "idle",
        Pulse = "pulse",
        Stall = "stall",
        TaskYield = "task-yield",
    }
}

named! {
    /// Where the periodic diagnostic's deadlines fall: one delay after each expiry, or on a shared grid of the delay.
    pub enum Alignment ("alignment") {
        Unaligned = "unaligned",
        Aligned = "aligned",
    }
}

/// Options of the timer workloads.
#[derive(Clone, Debug, PartialEq)]
pub struct TimerOptions {
    pub workload: TimerWorkload,
    pub backend: Backend,
    pub mode: Mode,
    pub tasks: usize,
    pub iterations: usize,
    pub workers: usize,
    /// How long an expiring wait lasts.
    pub delay: Duration,
}

/// Options of the Native scheduler scenarios.
#[derive(Clone, Debug, PartialEq)]
pub struct ScenarioOptions {
    pub workload: ScenarioWorkload,
    pub workers: usize,
    pub iterations: usize,
    /// The unit every duration is a multiple of.
    pub unit: Duration,
}

/// Options of the periodic timer diagnostic.
#[derive(Clone, Debug, PartialEq)]
pub struct PeriodicOptions {
    pub backend: NativeBackend,
    pub workers: usize,
    pub tasks: usize,
    pub delay: Duration,
    pub alignment: Alignment,
    /// The length of the measurement window, which follows a warmup of a third of it.
    pub measure: Duration,
    /// Multiply-add steps after each expiry.
    pub work: u64,
}

/// One process's workload and its options.
#[derive(Clone, Debug, PartialEq)]
pub enum Benchmark {
    Timer(TimerOptions),
    Scenario(ScenarioOptions),
    Periodic(PeriodicOptions),
}

impl Benchmark {
    /// Parse `--name value` options. Unknown, duplicate, incomplete and inapplicable options are errors, as are invalid values.
    // SECURITY: Counts and durations have no upper limit, so huge values can exhaust memory or run indefinitely. Arguments come only from the operator and the repository's scenario metadata, who can already run arbitrary code here, and every failure exits without a record.
    pub fn parse(arguments: impl IntoIterator<Item = String>) -> io::Result<Self> {
        let mut arguments = Arguments::parse(arguments)?;
        let workload = arguments
            .0
            .remove("workload")
            .unwrap_or_else(|| "refresh".to_owned());
        let benchmark = if workload == "periodic" {
            Self::Periodic(PeriodicOptions {
                backend: arguments.take("backend", NativeBackend::Native)?,
                workers: arguments.positive("workers", 4)?,
                tasks: arguments.positive("tasks", 1024)?,
                delay: Duration::from_micros(arguments.positive("delay-us", 1000)?),
                alignment: arguments.take("alignment", Alignment::Unaligned)?,
                measure: Duration::from_millis(arguments.positive("measure-ms", 3000)?),
                work: arguments.take("work", 0)?,
            })
        } else if let Ok(workload) = workload.parse() {
            Self::Scenario(ScenarioOptions {
                workload,
                workers: arguments.positive("workers", 1)?,
                iterations: arguments.positive("iterations", 100)?,
                unit: Duration::from_micros(arguments.positive("unit-us", 1000)?),
            })
        } else {
            Self::Timer(TimerOptions {
                workload: timer_workload(&workload)?,
                backend: arguments.take("backend", Backend::Native)?,
                mode: arguments.take("mode", Mode::After)?,
                tasks: arguments.positive("tasks", 1)?,
                iterations: arguments.positive("iterations", 100_000)?,
                workers: arguments.positive("workers", 1)?,
                delay: Duration::from_micros(arguments.take("delay-us", 1000)?),
            })
        };
        arguments.finish(&workload)?;
        Ok(benchmark)
    }

    /// Every option that changes the work, for the record's parameters, with the measurement features the binary was built with.
    pub fn parameters(&self) -> Value {
        let mut parameters = match self {
            Self::Timer(options) => json!({
                "workload": options.workload.name(),
                "backend": options.backend.name(),
                "mode": options.mode.name(),
                "tasks": options.tasks,
                "iterations": options.iterations,
                "workers": options.workers,
                "delay_us": options.delay.as_micros() as u64,
            }),
            Self::Scenario(options) => json!({
                "workload": options.workload.name(),
                "backend": "native",
                "workers": options.workers,
                "iterations": options.iterations,
                "unit_us": options.unit.as_micros() as u64,
            }),
            Self::Periodic(options) => json!({
                "workload": "periodic",
                "backend": options.backend.name(),
                "workers": options.workers,
                "tasks": options.tasks,
                "delay_us": options.delay.as_micros() as u64,
                "alignment": options.alignment.name(),
                "measure_ms": options.measure.as_millis() as u64,
                "work": options.work,
            }),
        };
        // Both features change timing, so a campaign must not compare a build with them to one without.
        parameters["features"] = json!(FEATURES);
        parameters
    }
}

/// The measurement features this binary was built with.
pub const FEATURES: &[&str] = &[
    #[cfg(feature = "allocations")]
    "allocations",
    #[cfg(feature = "diagnostics")]
    "diagnostics",
];

fn timer_workload(name: &str) -> io::Result<TimerWorkload> {
    #[cfg(not(feature = "allocations"))]
    if name == "alloc" {
        return Err(invalid(
            "the alloc workload needs the allocations feature, which replaces the global allocator",
        ));
    }
    name.parse()
}

// Every option of any family.
const OPTIONS: &[&str] = &[
    "workload",
    "backend",
    "mode",
    "tasks",
    "iterations",
    "workers",
    "delay-us",
    "unit-us",
    "alignment",
    "measure-ms",
    "work",
];

// Options by name, removed as each family takes the ones it uses, so that any left over are reported.
struct Arguments(BTreeMap<String, String>);

impl Arguments {
    fn parse(arguments: impl IntoIterator<Item = String>) -> io::Result<Self> {
        let mut values = BTreeMap::new();
        let mut arguments = arguments.into_iter();
        while let Some(argument) = arguments.next() {
            let Some(name) = argument.strip_prefix("--") else {
                return Err(invalid(format!(
                    "expected --name value, found {argument:?}"
                )));
            };
            let value = arguments
                .next()
                .ok_or_else(|| invalid(format!("missing value for --{name}")))?;
            if values.insert(name.to_owned(), value).is_some() {
                return Err(invalid(format!("duplicate option --{name}")));
            }
        }
        Ok(Self(values))
    }

    fn take<T: FromStr<Err: Display>>(&mut self, name: &str, default: T) -> io::Result<T> {
        self.0.remove(name).map_or(Ok(default), |value| {
            value
                .parse()
                .map_err(|error| invalid(format!("invalid --{name} {value:?}: {error}")))
        })
    }

    fn positive<T: FromStr<Err: Display> + Default + PartialEq>(
        &mut self,
        name: &str,
        default: T,
    ) -> io::Result<T> {
        let value = self.take(name, default)?;
        if value == T::default() {
            return Err(invalid(format!("--{name} must be positive")));
        }
        Ok(value)
    }

    fn finish(self, workload: &str) -> io::Result<()> {
        match self.0.into_keys().next() {
            Some(name) if !OPTIONS.contains(&name.as_str()) => {
                Err(invalid(format!("unknown option --{name}")))
            }
            Some(name) => Err(invalid(format!(
                "--{name} does not apply to the {workload} workload"
            ))),
            None => Ok(()),
        }
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

#[cfg(test)]
mod tests;
