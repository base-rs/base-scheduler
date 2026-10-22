// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use core_scheduler::{BufferLease, BufferPool, BufferSlice, FrozenBuffer};
use io_uring::{IoUring, opcode, types};
use std::{
    fs::File,
    io::{Read, Write},
    net::{TcpListener, UdpSocket},
    os::fd::{AsRawFd, OwnedFd},
    thread::JoinHandle,
    time::Instant,
};

const ALIGNMENT: usize = 4096;
const FILE_BYTES: usize = 8 * 1024 * 1024;
const WARMUP: usize = 32;

#[derive(Clone)]
struct Workload {
    fixed: bool,
    kind: String,
    write: bool,
    size: usize,
    depth: usize,
    iterations: usize,
    retained: bool,
    directory: String,
}
impl Workload {
    fn network(&self) -> bool {
        matches!(self.kind.as_str(), "tcp" | "udp")
    }
}

struct Region {
    base: usize,
    offset: usize,
}

struct Flight {
    buffer: BufferLease,
    offset: usize,
    transferred: usize,
    iteration: usize,
    writing: bool,
}

struct Driver {
    // Close the ring before releasing its registered pool or file owners.
    ring: IoUring,
    pool: BufferPool,
    regions: Vec<Region>,
    files: Vec<OwnedFd>,
    pending: Vec<Option<Flight>>,
    retained: Vec<Option<BufferSlice<FrozenBuffer>>>,
    work: Workload,
}
impl Drop for Driver {
    fn drop(&mut self) {
        // A failed assertion or syscall must not free kernel-accessible memory during unwinding. This experiment deliberately leaks any unretired owners; normal completion leaves every slot empty.
        for flight in self.pending.drain(..).flatten() {
            std::mem::forget(flight);
        }
    }
}

impl Driver {
    fn new(work: Workload) -> (Self, Vec<JoinHandle<()>>, f64) {
        let pool = BufferPool::new(work.depth * 2 + 1, work.size + ALIGNMENT - 1).unwrap();
        let mut leases: Vec<_> = (0..pool.capacity())
            .map(|_| pool.try_acquire().unwrap())
            .collect();
        let mut regions: Vec<_> = leases
            .iter_mut()
            .map(|buffer| {
                let base = buffer.as_mut().as_mut_ptr() as usize;
                let offset = base.wrapping_neg() & (ALIGNMENT - 1);
                if work.size % ALIGNMENT == 0 {
                    let pointer = (base + offset) as *mut libc::c_void;
                    // SAFETY: These whole pages are exclusively owned by this lease. Refault them without transparent huge pages in both modes so registration stays within the host's 8 MiB locked-memory limit. The entire range is reinitialized before use.
                    assert_eq!(
                        unsafe { libc::madvise(pointer, work.size, libc::MADV_NOHUGEPAGE) },
                        0
                    );
                    assert_eq!(
                        unsafe { libc::madvise(pointer, work.size, libc::MADV_DONTNEED) },
                        0
                    );
                }
                buffer.as_mut()[offset..offset + work.size].fill(42);
                Region { base, offset }
            })
            .collect();
        regions.sort_unstable_by_key(|region| region.base);
        let ring = IoUring::new((work.depth as u32).next_power_of_two().max(2)).unwrap();
        let started = Instant::now();
        if work.fixed {
            let iovecs: Vec<_> = regions
                .iter()
                .map(|region| libc::iovec {
                    iov_base: (region.base + region.offset) as *mut libc::c_void,
                    iov_len: work.size,
                })
                .collect();
            // SAFETY: The retained pool owns every stable allocation until after the ring closes. These page-aligned subranges stay initialized and are accessed only through exclusive in-flight leases.
            unsafe { ring.submitter().register_buffers(&iovecs) }.unwrap();
        }
        let registration_seconds = started.elapsed().as_secs_f64();
        drop(leases);
        let (files, peers) = if work.network() {
            network(&work)
        } else {
            let mut file = tempfile::tempfile_in(&work.directory).unwrap();
            let block = vec![if work.write { 17 } else { 42 }; 65536];
            for _ in 0..FILE_BYTES / block.len() {
                file.write_all(&block).unwrap();
            }
            file.sync_data().unwrap();
            if work.kind == "direct" {
                // SAFETY: fcntl receives a live descriptor and integer flags; no pointer arguments are involved.
                let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
                assert!(flags >= 0);
                assert_eq!(
                    unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFL, flags | libc::O_DIRECT) },
                    0
                );
            }
            (
                (0..work.depth)
                    .map(|_| file.try_clone().unwrap().into())
                    .collect(),
                Vec::new(),
            )
        };
        let pending = (0..work.depth).map(|_| None).collect();
        let retained = (0..work.depth).map(|_| None).collect();
        (
            Self {
                ring,
                pool,
                regions,
                files,
                pending,
                retained,
                work,
            },
            peers,
            registration_seconds,
        )
    }

    fn start(&mut self, slot: usize, iteration: usize) {
        let mut buffer = self.pool.try_acquire().unwrap();
        let base = buffer.as_mut().as_mut_ptr() as usize;
        let index = self
            .regions
            .binary_search_by_key(&base, |region| region.base)
            .unwrap();
        let offset = self.regions[index].offset;
        self.queue(
            slot,
            Flight {
                buffer,
                offset,
                transferred: 0,
                iteration,
                writing: self.work.network() || self.work.write,
            },
        );
    }

    fn queue(&mut self, slot: usize, flight: Flight) {
        self.pending[slot] = Some(flight);
        let flight = self.pending[slot].as_mut().unwrap();
        let base = flight.buffer.as_mut().as_mut_ptr();
        // Perform the same lookup in both modes so the kernel buffer strategy is the only timed difference.
        let index = self
            .regions
            .binary_search_by_key(&(base as usize), |region| region.base)
            .unwrap() as u16;
        // SAFETY: The exclusive lease remains in pending through the terminal CQE; this initialized subrange is within the registered allocation.
        let pointer = unsafe { base.add(flight.offset + flight.transferred) };
        let length = (self.work.size - flight.transferred) as u32;
        let descriptor = types::Fd(self.files[slot].as_raw_fd());
        let position = if self.work.network() {
            0
        } else {
            (((flight.iteration * self.work.depth + slot) * self.work.size) % FILE_BYTES
                + flight.transferred) as u64
        };
        let entry = match (self.work.fixed, flight.writing) {
            (false, false) => opcode::Read::new(descriptor, pointer, length)
                .offset(position)
                .build(),
            (false, true) => opcode::Write::new(descriptor, pointer, length)
                .offset(position)
                .build(),
            (true, false) => opcode::ReadFixed::new(descriptor, pointer, length, index)
                .offset(position)
                .build(),
            (true, true) => opcode::WriteFixed::new(descriptor, pointer, length, index)
                .offset(position)
                .build(),
        }
        .user_data(slot as u64);
        // SAFETY: The descriptor, pool allocation, and exclusive lease stay owned by this driver until completion. Error unwinding leaks unretired leases rather than releasing them.
        unsafe { self.ring.submission().push(&entry) }.unwrap();
    }

    fn run(&mut self, iterations: usize) -> serde_json::Value {
        let mut active = self.work.depth;
        let mut completions = Vec::with_capacity(active);
        let mut enters = 0u64;
        let mut cqes = 0u64;
        let started = Instant::now();
        for slot in 0..self.work.depth {
            self.start(slot, 0);
        }
        while active != 0 {
            self.ring.submit_and_wait(1).unwrap();
            enters += 1;
            completions.clear();
            completions.extend(
                self.ring
                    .completion()
                    .map(|entry| (entry.user_data() as usize, entry.result())),
            );
            cqes += completions.len() as u64;
            for &(slot, count) in &completions {
                assert!(
                    count > 0,
                    "I/O did not progress: {}",
                    std::io::Error::from_raw_os_error(-count)
                );
                let mut flight = self.pending[slot].take().unwrap();
                if self.work.kind == "udp" {
                    assert_eq!(count as usize, self.work.size);
                }
                flight.transferred += count as usize;
                assert!(flight.transferred <= self.work.size);
                if flight.transferred < self.work.size {
                    self.queue(slot, flight);
                    continue;
                }
                if self.work.network() && flight.writing {
                    flight.transferred = 0;
                    flight.writing = false;
                    self.queue(slot, flight);
                    continue;
                }
                if !flight.writing {
                    let bytes =
                        &flight.buffer.as_ref()[flight.offset..flight.offset + self.work.size];
                    assert_eq!(bytes[0], 42);
                    assert_eq!(bytes[self.work.size - 1], 42);
                    std::hint::black_box(bytes);
                }
                let iteration = flight.iteration + 1;
                if self.work.retained {
                    let frozen = flight.buffer.freeze();
                    self.retained[slot] =
                        Some(frozen.view(flight.offset..flight.offset + self.work.size));
                } else {
                    drop(flight);
                }
                if iteration < iterations {
                    self.start(slot, iteration);
                } else {
                    active -= 1;
                }
            }
        }
        let seconds = started.elapsed().as_secs_f64();
        let bytes = iterations * self.work.depth * self.work.size;
        serde_json::json!({"seconds":seconds,"bytes":bytes,"gib_per_second":bytes as f64 / seconds / 1073741824.0,"messages_per_second":(iterations*self.work.depth) as f64 / seconds,"enters":enters,"cqes":cqes})
    }

    fn verify_file(&self) {
        if !self.work.network() && self.work.write {
            let mut file = File::from(self.files[0].try_clone().unwrap());
            // SAFETY: There is no remaining I/O. Clear O_DIRECT so verification can use ordinary initialized storage.
            let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
            assert!(flags >= 0);
            assert_eq!(
                unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFL, flags & !libc::O_DIRECT) },
                0
            );
            let mut bytes = Vec::new();
            std::io::Seek::rewind(&mut file).unwrap();
            file.read_to_end(&mut bytes).unwrap();
            let written = (self.work.iterations.max(WARMUP) * self.work.depth * self.work.size)
                .min(FILE_BYTES);
            assert!(bytes[..written].iter().all(|&byte| byte == 42));
            assert!(bytes[written..].iter().all(|&byte| byte == 17));
        }
    }
}

fn network(work: &Workload) -> (Vec<OwnedFd>, Vec<JoinHandle<()>>) {
    let mut files = Vec::new();
    let mut peers = Vec::new();
    for _ in 0..work.depth {
        if work.kind == "tcp" {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let client = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
            let (mut server, _) = listener.accept().unwrap();
            client.set_nodelay(true).unwrap();
            server.set_nodelay(true).unwrap();
            server
                .set_read_timeout(Some(std::time::Duration::from_secs(10)))
                .unwrap();
            let size = work.size;
            let iterations = work.iterations + WARMUP;
            peers.push(std::thread::spawn(move || {
                let mut bytes = vec![0; size];
                for _ in 0..iterations {
                    server.read_exact(&mut bytes).unwrap();
                    server.write_all(&bytes).unwrap();
                }
            }));
            files.push(client.into());
        } else {
            let client = UdpSocket::bind("127.0.0.1:0").unwrap();
            let server = UdpSocket::bind("127.0.0.1:0").unwrap();
            client.connect(server.local_addr().unwrap()).unwrap();
            server.connect(client.local_addr().unwrap()).unwrap();
            server
                .set_read_timeout(Some(std::time::Duration::from_secs(10)))
                .unwrap();
            let size = work.size;
            let iterations = work.iterations + WARMUP;
            peers.push(std::thread::spawn(move || {
                let mut bytes = vec![0; size];
                for _ in 0..iterations {
                    assert_eq!(server.recv(&mut bytes).unwrap(), size);
                    assert_eq!(server.send(&bytes).unwrap(), size);
                }
            }));
            files.push(client.into());
        }
    }
    (files, peers)
}

pub(super) fn execute(args: &[String]) -> serde_json::Value {
    let mode = args.get(1).map(String::as_str).unwrap_or("fixed");
    assert!(matches!(mode, "ordinary" | "fixed"));
    let kind = args.get(2).cloned().unwrap_or("tcp".into());
    assert!(matches!(kind.as_str(), "tcp" | "udp" | "cached" | "direct"));
    let direction = args.get(3).map(String::as_str).unwrap_or("read");
    assert!(matches!(direction, "read" | "write"));
    let size: usize = args.get(4).map(|s| s.parse().unwrap()).unwrap_or(4096);
    let depth: usize = args.get(5).map(|s| s.parse().unwrap()).unwrap_or(1);
    let iterations: usize = args.get(6).map(|s| s.parse().unwrap()).unwrap_or(10000);
    let ownership = args.get(7).map(String::as_str).unwrap_or("pooled");
    assert!(matches!(ownership, "pooled" | "retained"));
    assert!(size > 0 && size <= 131072 && depth > 0 && depth <= 16 && iterations > 0);
    if kind == "udp" {
        assert!(size <= 65507);
    }
    if matches!(kind.as_str(), "cached" | "direct") {
        assert_eq!(size % ALIGNMENT, 0);
    }
    let work = Workload {
        fixed: mode == "fixed",
        kind: kind.clone(),
        write: direction == "write",
        size,
        depth,
        iterations,
        retained: ownership == "retained",
        directory: args.get(8).cloned().unwrap_or(".".into()),
    };
    let (mut driver, peers, registration_seconds) = Driver::new(work);
    driver.run(WARMUP);
    let mut result = driver.run(iterations);
    driver.verify_file();
    assert!(driver.pending.iter().all(Option::is_none));
    let started = Instant::now();
    if driver.work.fixed {
        // Release the table explicitly: ring destruction can defer resource cleanup across short-lived benchmark processes, exhausting the per-user locked-memory budget.
        driver.ring.submitter().unregister_buffers().unwrap();
    }
    let unregistration_seconds = started.elapsed().as_secs_f64();
    drop(driver);
    for peer in peers {
        peer.join().unwrap();
    }
    result["mode"] = mode.into();
    result["kind"] = kind.into();
    result["direction"] = direction.into();
    result["size"] = size.into();
    result["depth"] = depth.into();
    result["iterations"] = iterations.into();
    result["ownership"] = ownership.into();
    result["registration_seconds"] = registration_seconds.into();
    result["unregistration_seconds"] = unregistration_seconds.into();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_and_ordinary_buffers_preserve_data() {
        for mode in ["ordinary", "fixed"] {
            for kind in ["tcp", "udp", "cached", "direct"] {
                for direction in ["read", "write"] {
                    for ownership in ["pooled", "retained"] {
                        let args = [
                            "registered",
                            mode,
                            kind,
                            direction,
                            "4096",
                            "2",
                            "3",
                            ownership,
                        ]
                        .map(str::to_owned);
                        let result = execute(&args);
                        assert_eq!(result["bytes"], 4096 * 2 * 3);
                        assert!(result["gib_per_second"].as_f64().unwrap() > 0.0);
                    }
                }
            }
        }
    }

    #[test]
    fn stream_transfers_reassemble_partial_completions() {
        for mode in ["ordinary", "fixed"] {
            let args = [
                "registered",
                mode,
                "tcp",
                "read",
                "131072",
                "2",
                "8",
                "retained",
            ]
            .map(str::to_owned);
            assert_eq!(execute(&args)["bytes"], 131072 * 2 * 8);
        }
    }

    #[test]
    fn small_network_payloads_do_not_discard_allocator_pages() {
        for mode in ["ordinary", "fixed"] {
            for (kind, size) in [("tcp", "64"), ("udp", "1200")] {
                let args = ["registered", mode, kind, "read", size, "1", "3", "retained"]
                    .map(str::to_owned);
                assert_eq!(execute(&args)["iterations"], 3);
            }
        }
    }

    #[test]
    #[should_panic(expected = "I/O did not progress")]
    fn unexpected_end_of_file_rejects_a_truncated_sample() {
        let work = Workload {
            fixed: true,
            kind: "cached".into(),
            write: false,
            size: 4096,
            depth: 1,
            iterations: 1,
            retained: false,
            directory: ".".into(),
        };
        let (mut driver, peers, _) = Driver::new(work);
        assert!(peers.is_empty());
        File::from(driver.files[0].try_clone().unwrap())
            .set_len(0)
            .unwrap();
        driver.run(1);
    }

    #[test]
    fn uncollected_requests_retain_their_pool_storage_during_unwinding() {
        let work = Workload {
            fixed: true,
            kind: "cached".into(),
            write: false,
            size: 4096,
            depth: 1,
            iterations: 1,
            retained: false,
            directory: ".".into(),
        };
        let (mut driver, peers, _) = Driver::new(work);
        assert!(peers.is_empty());
        let pool = driver.pool.clone();
        driver.start(0, 0);
        driver.ring.submit().unwrap();
        drop(driver);
        let leases: Vec<_> = std::iter::from_fn(|| pool.try_acquire()).collect();
        assert_eq!(leases.len(), pool.capacity() - 1);
    }
}
