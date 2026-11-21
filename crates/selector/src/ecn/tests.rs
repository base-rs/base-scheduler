// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use socket2::{Domain, Protocol};

fn datagram(domain: Domain) -> Socket {
    Socket::new(domain, Type::DGRAM, Some(Protocol::UDP)).unwrap()
}

#[test]
fn streams_are_rejected_before_support() {
    let stream = Socket::new(Domain::IPV4, Type::STREAM, None).unwrap();
    for result in [
        set_receive(&stream, true),
        receive(&stream).map(drop),
        set_send(&stream, Ecn::Ect0),
        send(&stream).map(drop),
    ] {
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidInput);
    }
    crate::socket_io::SOCKET_TYPE_ERROR.with(|slot| slot.set(Some(io::ErrorKind::NotFound)));
    assert_eq!(
        receive(&datagram(Domain::IPV4)).unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
}

#[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos")))]
#[test]
fn unsupported_platforms_reject_packet_sockets() {
    let socket = datagram(Domain::IPV4);
    for result in [
        set_receive(&socket, true),
        receive(&socket).map(drop),
        set_send(&socket, Ecn::Ect0),
        send(&socket).map(drop),
    ] {
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Unsupported);
    }
}

#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
mod supported {
    use super::*;
    use socket2::SockAddr;
    use std::net::SocketAddr;

    fn dual_stack(dual: bool) -> Socket {
        let socket = datagram(Domain::IPV6);
        socket.set_only_v6(!dual).unwrap();
        socket
    }

    #[cfg(unix)]
    #[test]
    fn non_ip_sockets_are_rejected() {
        let socket = Socket::new(Domain::UNIX, Type::DGRAM, None).unwrap();
        assert_eq!(
            receive(&socket).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn receive_follows_the_kernel_option() {
        let ipv4 = datagram(Domain::IPV4);
        assert!(!receive(&ipv4).unwrap());
        set_receive(&ipv4, true).unwrap();
        assert!(receive(&ipv4).unwrap() && ipv4.recv_tos_v4().unwrap());
        ipv4.set_recv_tos_v4(false).unwrap();
        assert!(!receive(&ipv4).unwrap());

        let ipv6 = dual_stack(false);
        set_receive(&ipv6, true).unwrap();
        assert!(ipv6.recv_tclass_v6().unwrap());
        #[cfg(any(target_os = "linux", target_os = "android"))]
        assert!(
            !ipv6.recv_tos_v4().unwrap(),
            "an IPv6-only socket configured IPv4"
        );
        assert!(receive(&ipv6).unwrap());
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn dual_stack_receive_configures_both_sides_and_gets_the_ipv6_option() {
        let socket = dual_stack(true);
        set_receive(&socket, true).unwrap();
        assert!(socket.recv_tclass_v6().unwrap() && socket.recv_tos_v4().unwrap());
        socket.set_recv_tclass_v6(false).unwrap();
        assert!(!receive(&socket).unwrap());
        set_receive(&socket, false).unwrap();
        assert!(!socket.recv_tos_v4().unwrap());
    }

    #[test]
    fn send_preserves_dscp() {
        let ipv4 = datagram(Domain::IPV4);
        ipv4.set_tos_v4(0xb8).unwrap();
        set_send(&ipv4, Ecn::Ect0).unwrap();
        assert_eq!(ipv4.tos_v4().unwrap(), 0xba);
        assert_eq!(send(&ipv4).unwrap(), Ecn::Ect0);
        set_send(&ipv4, Ecn::NotEct).unwrap();
        assert_eq!(ipv4.tos_v4().unwrap(), 0xb8);

        let ipv6 = dual_stack(false);
        ipv6.set_tclass_v6(0x28).unwrap();
        set_send(&ipv6, Ecn::Ce).unwrap();
        assert_eq!(ipv6.tclass_v6().unwrap(), 0x2b);
        #[cfg(any(target_os = "linux", target_os = "android"))]
        assert_eq!(
            ipv6.tos_v4().unwrap() & 0b11,
            0,
            "an IPv6-only socket marked IPv4"
        );
        assert_eq!(send(&ipv6).unwrap(), Ecn::Ce);
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn dual_stack_send_marks_both_sides_and_gets_the_ipv6_option() {
        let socket = dual_stack(true);
        socket.set_tos_v4(0x20).unwrap();
        set_send(&socket, Ecn::Ect1).unwrap();
        assert_eq!(socket.tclass_v6().unwrap() & 0b11, 0b01);
        assert_eq!(socket.tos_v4().unwrap(), 0x21);
        socket.set_tclass_v6(0b10).unwrap();
        assert_eq!(send(&socket).unwrap(), Ecn::Ect0);
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn failing(index: usize, call: impl FnOnce() -> io::Result<()>) {
        OPTION_FAILURE.with(|slot| slot.set(Some((index, io::ErrorKind::PermissionDenied))));
        assert_eq!(call().unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert!(OPTION_FAILURE.with(|slot| slot.get()).is_none());
    }

    // A dual-stack set that fails part-way returns the error and leaves the calls already made in place, and gets report that kernel state. Each set first reads the address and `IPV6_V6ONLY`.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn dual_stack_failures_leave_the_kernel_state() {
        let socket = dual_stack(true);
        // The second option call: `IP_RECVTOS`, after `IPV6_RECVTCLASS`.
        failing(3, || set_receive(&socket, true));
        assert!(receive(&socket).unwrap());
        assert!(!socket.recv_tos_v4().unwrap());

        // Option calls: read and set the traffic class, then read and set the TOS.
        failing(4, || set_send(&socket, Ecn::Ce));
        assert_eq!(send(&socket).unwrap(), Ecn::Ce);
        assert_eq!(socket.tos_v4().unwrap() & 0b11, 0);
        failing(2, || set_send(&socket, Ecn::NotEct));
        assert_eq!(send(&socket).unwrap(), Ecn::Ce);
    }

    // Every system call of every set and get fails in turn, and the operation returns the failure.
    // On macOS a dual-stack socket is configured through its IPv6 options alone.
    #[cfg(target_os = "macos")]
    #[test]
    fn dual_stack_sockets_use_the_ipv6_options() {
        let socket = dual_stack(true);
        set_receive(&socket, true).unwrap();
        assert!(socket.recv_tclass_v6().unwrap() && receive(&socket).unwrap());
        set_send(&socket, Ecn::Ce).unwrap();
        assert_eq!(socket.tclass_v6().unwrap() & 0b11, 0b11);
        assert_eq!(send(&socket).unwrap(), Ecn::Ce);
    }

    #[test]
    fn every_call_failure_is_returned() {
        for socket in [datagram(Domain::IPV4), dual_stack(false), dual_stack(true)] {
            let operations: [&dyn Fn() -> io::Result<()>; 4] = [
                &|| set_receive(&socket, true),
                &|| receive(&socket).map(drop),
                &|| set_send(&socket, Ecn::Ect0),
                &|| send(&socket).map(drop),
            ];
            for operation in operations {
                for index in 0.. {
                    OPTION_FAILURE
                        .with(|slot| slot.set(Some((index, io::ErrorKind::PermissionDenied))));
                    let result = operation();
                    // An index past the operation's last call is never reached.
                    if OPTION_FAILURE.with(|slot| slot.take()).is_some() {
                        result.unwrap();
                        assert!(index > 0);
                        break;
                    }
                    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
                }
            }
        }
    }

    // One control message: a header, then data padded to `CMSG_SPACE`.
    fn message(level: libc::c_int, kind: libc::c_int, data: &[u8]) -> Vec<u8> {
        // SAFETY: Length arithmetic only.
        let (length, space) = unsafe {
            (
                libc::CMSG_LEN(data.len() as u32) as usize,
                libc::CMSG_SPACE(data.len() as u32) as usize,
            )
        };
        let mut bytes = vec![0; space];
        // SAFETY: An all-zero `cmsghdr` is valid, and the write is in bounds.
        let mut header: libc::cmsghdr = unsafe { std::mem::zeroed() };
        header.cmsg_len = length as _;
        header.cmsg_level = level;
        header.cmsg_type = kind;
        unsafe { std::ptr::write_unaligned(bytes.as_mut_ptr().cast(), header) };
        let offset = unsafe { libc::CMSG_LEN(0) } as usize;
        bytes[offset..offset + data.len()].copy_from_slice(data);
        bytes
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    const IPV4: libc::c_int = libc::IP_TOS;
    #[cfg(target_os = "macos")]
    const IPV4: libc::c_int = libc::IP_RECVTOS;

    fn class(value: libc::c_int) -> Vec<u8> {
        message(libc::IPPROTO_IPV6, libc::IPV6_TCLASS, &value.to_ne_bytes())
    }

    #[test]
    fn parses_both_forms() {
        assert_eq!(
            parse(&message(libc::IPPROTO_IP, IPV4, &[0xb9]), false),
            Some(Ecn::Ect1)
        );
        assert_eq!(parse(&class(0x2a), false), Some(Ecn::Ect0));
        assert_eq!(
            parse(
                &message(libc::IPPROTO_IP, IPV4, &0x23_i32.to_ne_bytes()),
                false
            ),
            Some(Ecn::Ce)
        );
        assert_eq!(parse(&class(0), false), Some(Ecn::NotEct));
        assert_eq!(parse(&[], false), None);
        assert!(class(0).len() <= CONTROL_SPACE);
    }

    #[test]
    fn truncation_reports_none_even_with_an_intact_marking() {
        assert_eq!(parse(&class(0x2a), true), None);
    }

    #[test]
    fn unrelated_and_malformed_control_data() {
        let unrelated = message(libc::SOL_SOCKET, libc::SCM_RIGHTS, &[0; 4]);
        assert_eq!(parse(&unrelated, false), None);
        let mut both = unrelated.clone();
        both.extend(class(0x03));
        assert_eq!(parse(&both, false), Some(Ecn::Ce));
        // A marking of the wrong size is skipped.
        assert_eq!(
            parse(
                &message(libc::IPPROTO_IPV6, libc::IPV6_TCLASS, &[1, 2]),
                false
            ),
            None
        );
        // A header shorter than itself, or longer than the data.
        let mut short = class(2);
        short[..std::mem::size_of::<libc::cmsghdr>()].copy_from_slice(&header_bytes(1));
        assert_eq!(parse(&short, false), None);
        let mut long = class(2);
        long[..std::mem::size_of::<libc::cmsghdr>()].copy_from_slice(&header_bytes(1024));
        assert_eq!(parse(&long, false), None);
        // A partial header is ignored.
        assert_eq!(parse(&class(2)[..4], false), None);
    }

    fn header_bytes(length: usize) -> Vec<u8> {
        let mut bytes = class(2);
        // SAFETY: As in `message`.
        let mut header: libc::cmsghdr = unsafe { std::ptr::read_unaligned(bytes.as_ptr().cast()) };
        header.cmsg_len = length as _;
        unsafe { std::ptr::write_unaligned(bytes.as_mut_ptr().cast(), header) };
        bytes.truncate(std::mem::size_of::<libc::cmsghdr>());
        bytes
    }

    // Loopback preserves the TOS byte, so a marked datagram reports its marking through the receive path, and an unmarked or unreported one reports none.
    #[test]
    fn packet_receives_report_the_marking() {
        let receiver = datagram(Domain::IPV4);
        receiver
            .bind(&SockAddr::from(
                "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
            ))
            .unwrap();
        let address = receiver.local_addr().unwrap();
        let sender = datagram(Domain::IPV4);
        let receive = |kind| {
            let mut buffer = [0u8; 4];
            let mut vectors = [std::io::IoSliceMut::new(&mut buffer)];
            crate::socket_io::recvmsg_poll(
                &receiver,
                &mut vectors,
                base_scheduler::SocketRecvFlags::default(),
                kind,
            )
        };
        let wait = || {
            crate::socket_io::wait(&receiver, crate::Interest::Readable).unwrap();
        };
        set_send(&sender, Ecn::Ect0).unwrap();
        sender.send_to(b"a", &address).unwrap();
        wait();
        assert_eq!(
            receive(crate::socket_io::Receive::Packet).unwrap().ecn,
            None
        );
        set_receive(&receiver, true).unwrap();
        sender.send_to(b"b", &address).unwrap();
        wait();
        assert_eq!(
            receive(crate::socket_io::Receive::Packet).unwrap().ecn,
            Some(Ecn::Ect0)
        );
        sender.send_to(b"c", &address).unwrap();
        wait();
        assert_eq!(
            receive(crate::socket_io::Receive::Stream).unwrap().ecn,
            None
        );
    }
}

// The conformance suite over loopback, for every family. Native forwards its packet receives and options to this selector.
mod conformance {
    use crate::{ReadinessSelector, test_support};
    use base_scheduler_conformance::{EcnFamily, LoopbackEcnFixture};
    use futures_lite::future::block_on;

    fn run(
        family: EcnFamily,
        test: impl AsyncFnOnce(&LoopbackEcnFixture<'_, ReadinessSelector>) + Send + 'static,
    ) {
        test_support::run(move || {
            let scheduler = test_support::scheduler();
            #[cfg(unix)]
            let fixture = block_on(LoopbackEcnFixture::open(&scheduler, family, |socket| {
                std::os::fd::AsRawFd::as_raw_fd(socket.get_ref())
            }));
            #[cfg(not(unix))]
            let fixture = block_on(LoopbackEcnFixture::open(&scheduler, family));
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| block_on(test(&fixture))));
            block_on(fixture.close());
            if let Err(payload) = result {
                std::panic::resume_unwind(payload);
            }
        });
    }

    macro_rules! family {
        ($name:ident, $family:ident, $suite:ident) => {
            mod $name {
                use super::*;
                fn run(
                    test: impl AsyncFnOnce(&LoopbackEcnFixture<'_, ReadinessSelector>)
                    + Send
                    + 'static,
                ) {
                    super::run(EcnFamily::$family, test);
                }
                base_scheduler_conformance::$suite!(run: run);
            }
        };
    }

    #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
    family!(ipv4, Ipv4, packet_ecn_tests);
    #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
    family!(ipv6, Ipv6, packet_ecn_tests);
    #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
    family!(dual_stack, DualStack, packet_ecn_tests);
    #[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos")))]
    family!(unsupported_ipv4, Ipv4, packet_ecn_unsupported_tests);
    #[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos")))]
    family!(unsupported_ipv6, Ipv6, packet_ecn_unsupported_tests);
}
