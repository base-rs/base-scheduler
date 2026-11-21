// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

/// Generate separately reported packet ECN tests. The runner supplies a fresh `PacketEcnFixture` reference per case.
///
/// Invoke inside a dedicated module for each family where the backend supports ECN: IPv4, IPv6 and dual-stack; use `packet_ecn_unsupported_tests!` elsewhere. The runner awaits the async closure exactly once, enforces deadlines, and propagates panics.
#[macro_export]
macro_rules! packet_ecn_tests {
    (run: $run:path $(,)?) => {
        $crate::__conformance_test_cases! { $run;
            markings_reach_every_receive_form => |fixture| {
                $crate::packet_ecn::markings(fixture).await;
            },
            reporting_follows_options_set_after_construction => |fixture| {
                $crate::packet_ecn::after_construction(fixture).await;
            },
            disabled_reporting_reports_none => |fixture| {
                $crate::packet_ecn::disabled(fixture).await;
            },
            native_reporting_reports_the_marking => |fixture| {
                $crate::packet_ecn::native_reporting(fixture).await;
            },
            send_preserves_dscp => |fixture| {
                $crate::packet_ecn::dscp(fixture).await;
            },
            gets_read_the_kernel_options => |fixture| {
                $crate::packet_ecn::gets(fixture).await;
            },
            streams_reject_the_options => |fixture| {
                $crate::packet_ecn::streams(fixture).await;
            },
        }
    };
}

/// Generate the packet ECN tests for a backend or platform without ECN support. The runner supplies a fresh `EcnOptionFixture` reference per case.
#[macro_export]
macro_rules! packet_ecn_unsupported_tests {
    (run: $run:path $(,)?) => {
        $crate::__conformance_test_cases! { $run;
            packet_sockets_report_unsupported => |fixture| {
                $crate::packet_ecn::unsupported(fixture).await;
            },
            streams_reject_the_options => |fixture| {
                $crate::packet_ecn::streams(fixture).await;
            },
        }
    };
}
