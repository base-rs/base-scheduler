// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use base_scheduler::{BufferLease, BufferSlice, Cancellation, FrozenBuffer, HeapBufferPool, IoBuf};
use io_uring::{IoUring, cqueue, opcode, types};
use std::{
    collections::VecDeque,
    io::{Read, Write},
    net::{TcpListener, UdpSocket},
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    sync::{Arc, mpsc},
    thread::JoinHandle,
    time::{Duration, Instant},
};

const HEADER: usize = 16;
const ALIGNMENT: usize = 4096;
const REPORT_USAGE: u16 = 1 << 3;
const COPIED: u32 = 1 << 31;
type View = BufferSlice<FrozenBuffer>;

#[derive(Clone)]
struct Workload {
    mode: String,
    transport: String,
    size: usize,
    depth: usize,
    iterations: usize,
    retained: bool,
    handoff: bool,
    listen: Option<String>,
}
impl Workload {
    fn registered(&self) -> bool {
        matches!(self.mode.as_str(), "registered-copy" | "fixed-zc")
    }
    fn zero_copy(&self) -> bool {
        matches!(self.mode.as_str(), "zc" | "fixed-zc")
    }
}

struct Region {
    base: usize,
    offset: usize,
}
struct Storage {
    pool: HeapBufferPool,
    regions: Vec<Region>,
}
impl Storage {
    fn new(work: &Workload) -> Arc<Self> {
        let pool = HeapBufferPool::new(work.depth * 4 + 1, work.size + ALIGNMENT - 1).unwrap();
        let mut leases: Vec<_> = (0..pool.capacity())
            .map(|_| pool.try_acquire().unwrap())
            .collect();
        let mut regions: Vec<_> = leases
            .iter_mut()
            .map(|lease| {
                let base = lease.as_mut().as_mut_ptr() as usize;
                let offset = base.wrapping_neg() & (ALIGNMENT - 1);
                if work.size % ALIGNMENT == 0 {
                    let pointer = (base + offset) as *mut libc::c_void;
                    // SAFETY: Only full pages wholly within this exclusive allocation are discarded. They are reinitialized before any read or registration.
                    assert_eq!(
                        unsafe { libc::madvise(pointer, work.size, libc::MADV_NOHUGEPAGE) },
                        0
                    );
                    assert_eq!(
                        unsafe { libc::madvise(pointer, work.size, libc::MADV_DONTNEED) },
                        0
                    );
                }
                lease.as_mut()[offset..offset + work.size].fill(0);
                Region { base, offset }
            })
            .collect();
        regions.sort_unstable_by_key(|region| region.base);
        drop(leases);
        Arc::new(Self { pool, regions })
    }
    fn offset(&self, buffer: &BufferLease) -> usize {
        let base = buffer.as_ref().as_ptr() as usize;
        self.regions[self
            .regions
            .binary_search_by_key(&base, |region| region.base)
            .unwrap()]
        .offset
    }
    fn index(&self, view: &[u8]) -> u16 {
        let address = view.as_ptr() as usize;
        let index = self
            .regions
            .partition_point(|region| region.base <= address)
            - 1;
        let region = &self.regions[index];
        assert!(address >= region.base + region.offset);
        assert!(address + view.len() <= region.base + self.pool.buffer_size());
        index as u16
    }
    fn register(&self, ring: &IoUring, size: usize) {
        let buffers: Vec<_> = self
            .regions
            .iter()
            .map(|region| libc::iovec {
                iov_base: (region.base + region.offset) as *mut libc::c_void,
                iov_len: size,
            })
            .collect();
        // SAFETY: Every ring retains this storage through explicit table unregistration. Writable access requires a unique pool lease; outstanding sends retain immutable views until their terminal CQE.
        unsafe { ring.submitter().register_buffers(&buffers) }.unwrap();
    }
}

struct Sending {
    connection: usize,
    buffer: View,
}
enum Flight {
    Receive {
        connection: usize,
        buffer: BufferLease,
        filled: usize,
    },
    Send(Sending),
    Notification(View),
}
#[derive(Default)]
struct Counters {
    enters: u64,
    cqes: u64,
    sends: u64,
    notifications: u64,
    copied_notifications: u64,
    copied_bytes: u64,
    short_sends: u64,
    pool_stalls: u64,
}
impl Counters {
    fn combine(&mut self, other: Self) {
        self.enters += other.enters;
        self.cqes += other.cqes;
        self.sends += other.sends;
        self.notifications += other.notifications;
        self.copied_notifications += other.copied_notifications;
        self.copied_bytes += other.copied_bytes;
        self.short_sends += other.short_sends;
        self.pool_stalls += other.pool_stalls;
    }
}

struct Driver {
    // Registered storage and native sockets outlive the ring. Unwinding deliberately leaks unretired leases rather than releasing kernel-accessible bytes.
    ring: IoUring,
    storage: Arc<Storage>,
    sockets: Arc<Vec<OwnedFd>>,
    pending: Vec<Option<Flight>>,
    free: Vec<usize>,
    active: usize,
    counters: Counters,
    work: Workload,
}
impl Drop for Driver {
    fn drop(&mut self) {
        for flight in self.pending.drain(..).flatten() {
            std::mem::forget(flight);
        }
    }
}
impl Driver {
    fn new(work: Workload, storage: Arc<Storage>, sockets: Arc<Vec<OwnedFd>>) -> Self {
        let capacity = (storage.pool.capacity() * 2).next_power_of_two();
        Self {
            ring: IoUring::new(capacity as u32).unwrap(),
            storage,
            sockets,
            pending: (0..capacity).map(|_| None).collect(),
            free: (0..capacity).rev().collect(),
            active: 0,
            counters: Counters::default(),
            work,
        }
    }
    fn queue(&mut self, flight: Flight) {
        let id = self
            .free
            .pop()
            .expect("request capacity covers all buffer owners");
        self.pending[id] = Some(flight);
        self.active += 1;
        let entry = match self.pending[id].as_mut().unwrap() {
            Flight::Receive {
                connection,
                buffer,
                filled,
            } => {
                let offset = self.storage.offset(buffer);
                let bytes = &mut buffer.as_mut()[offset + *filled..offset + self.work.size];
                opcode::Recv::new(
                    types::Fd(self.sockets[*connection].as_raw_fd()),
                    bytes.as_mut_ptr(),
                    bytes.len() as u32,
                )
                .flags(if self.work.transport == "tcp" {
                    libc::MSG_WAITALL
                } else {
                    0
                })
                .build()
            }
            Flight::Send(sending) => {
                let bytes = sending.buffer.as_ref();
                // Keep the same table lookup in every mode, isolating the kernel send strategy.
                let index = self.storage.index(bytes);
                let fd = types::Fd(self.sockets[sending.connection].as_raw_fd());
                self.counters.sends += 1;
                if self.work.zero_copy() {
                    opcode::SendZc::new(fd, bytes.as_ptr(), bytes.len() as u32)
                        .buf_index(if self.work.registered() {
                            Some(index)
                        } else {
                            None
                        })
                        .flags(libc::MSG_NOSIGNAL)
                        .zc_flags(REPORT_USAGE)
                        .build()
                } else {
                    opcode::Send::new(fd, bytes.as_ptr(), bytes.len() as u32)
                        .flags(libc::MSG_NOSIGNAL)
                        .build()
                }
            }
            Flight::Notification(_) => unreachable!("notifications are never submitted"),
        }
        .user_data(id as u64);
        // SAFETY: The request owns stable initialized memory and a live socket. Receive owners remain exclusive; send owners cannot recycle before all promised notification CQEs retire.
        unsafe { self.ring.submission().push(&entry) }.unwrap();
    }
    fn collect(&mut self, completions: &mut Vec<(usize, i32, u32)>) {
        self.ring.submit_and_wait(1).unwrap();
        self.counters.enters += 1;
        completions.clear();
        completions.extend(
            self.ring
                .completion()
                .map(|entry| (entry.user_data() as usize, entry.result(), entry.flags())),
        );
        self.counters.cqes += completions.len() as u64;
    }
    fn complete_send(&mut self, id: usize, result: i32, flags: u32) {
        if cqueue::notif(flags) {
            assert!(!cqueue::more(flags));
            self.counters.notifications += 1;
            if result as u32 & COPIED != 0 {
                self.counters.copied_notifications += 1;
            } else {
                assert!(result >= 0);
                self.counters.copied_bytes += result as u64;
            }
            let Flight::Notification(buffer) = self.pending[id].take().unwrap() else {
                unreachable!("notification without send result")
            };
            drop(buffer);
            self.release(id);
            return;
        }
        // Inspect failure before extracting ownership: unwinding must retain a kernel-accessible request if MORE promises a notification.
        assert!(
            result > 0,
            "send failed: {}",
            std::io::Error::from_raw_os_error(-result)
        );
        let Flight::Send(mut sending) = self.pending[id].take().unwrap() else {
            unreachable!("send result without request")
        };
        let count = result as usize;
        assert!(count <= sending.buffer.as_ref().len());
        let retry = if count < sending.buffer.as_ref().len() {
            assert_eq!(self.work.transport, "tcp", "datagrams must remain whole");
            self.counters.short_sends += 1;
            let pointer = sending.buffer.as_ref().as_ptr() as usize;
            let length = sending.buffer.as_ref().len();
            let owner = sending.buffer.into_inner();
            let start = pointer - owner.as_ref().as_ptr() as usize;
            let retry = Sending {
                connection: sending.connection,
                buffer: owner.view(start + count..start + length),
            };
            sending.buffer = owner.slice(start..start + length);
            Some(retry)
        } else {
            None
        };
        if cqueue::more(flags) {
            assert!(self.work.zero_copy());
            self.pending[id] = Some(Flight::Notification(sending.buffer));
        } else {
            drop(sending);
            self.release(id);
        }
        if let Some(retry) = retry {
            self.queue(Flight::Send(retry));
        }
    }
    fn release(&mut self, id: usize) {
        self.active -= 1;
        self.free.push(id);
    }
    fn unregister(&self) {
        assert_eq!(self.active, 0);
        if self.work.registered() {
            self.ring.submitter().unregister_buffers().unwrap();
        }
    }
    fn sender(mut self, input: mpsc::Receiver<Sending>, interrupt: Arc<OwnedFd>) -> Counters {
        let mut completions = Vec::with_capacity(self.pending.len());
        let mut armed = false;
        loop {
            let disconnected = loop {
                match input.try_recv() {
                    Ok(sending) => self.queue(Flight::Send(sending)),
                    Err(mpsc::TryRecvError::Empty) => break false,
                    Err(mpsc::TryRecvError::Disconnected) => break true,
                }
            };
            if disconnected && self.active == 0 {
                break;
            }
            if !armed {
                let entry =
                    opcode::PollAdd::new(types::Fd(interrupt.as_raw_fd()), libc::POLLIN as u32)
                        .build()
                        .user_data(u64::MAX);
                // SAFETY: The worker retains the eventfd until after this ring is destroyed; PollAdd accesses no application payload memory.
                unsafe { self.ring.submission().push(&entry) }.unwrap();
                armed = true;
            }
            self.collect(&mut completions);
            for &(id, result, flags) in &completions {
                if id == usize::MAX {
                    assert!(result > 0);
                    let mut value = 0u64;
                    // SAFETY: This initialized u64 has room for the eventfd's eight-byte counter. There is one reader and a successful poll preceded this read.
                    assert_eq!(
                        unsafe {
                            libc::read(interrupt.as_raw_fd(), (&mut value as *mut u64).cast(), 8)
                        },
                        8
                    );
                    armed = false;
                } else {
                    self.complete_send(id, result, flags);
                }
            }
        }
        self.unregister();
        std::mem::take(&mut self.counters)
    }
}

struct Output {
    channel: Option<mpsc::SyncSender<Sending>>,
    interrupt: Arc<OwnedFd>,
}
impl Output {
    fn notify(&self) {
        let value = 1u64;
        // SAFETY: The live eventfd reads exactly eight initialized bytes synchronously. The bounded channel limits outstanding increments far below counter saturation.
        assert_eq!(
            unsafe { libc::write(self.interrupt.as_raw_fd(), (&value as *const u64).cast(), 8) },
            8
        );
    }
    fn send(&self, sending: Sending) {
        self.channel.as_ref().unwrap().send(sending).unwrap();
        self.notify();
    }
}
impl Drop for Output {
    fn drop(&mut self) {
        drop(self.channel.take());
        self.notify();
    }
}

fn receive(mut driver: Driver, output: Option<Output>) -> Counters {
    let mut waiting: VecDeque<_> = (0..driver.work.depth).collect();
    let mut received = vec![0; driver.work.depth];
    let mut retained: Vec<Option<View>> = (0..driver.work.depth).map(|_| None).collect();
    let mut completions = Vec::with_capacity(driver.pending.len());
    while !waiting.is_empty() || driver.active != 0 {
        while let Some(&connection) = waiting.front() {
            let buffer = if let Some(buffer) = driver.storage.pool.try_acquire() {
                buffer
            } else {
                driver.counters.pool_stalls += 1;
                if driver.active != 0 {
                    break;
                }
                // With no requests on this ring, capacity must return from independent owners: application processing or the remote sender's notifications.
                futures_lite::future::block_on(driver.storage.pool.acquire(&Cancellation::never()))
                    .unwrap()
            };
            waiting.pop_front();
            driver.queue(Flight::Receive {
                connection,
                buffer,
                filled: 0,
            });
        }
        driver.collect(&mut completions);
        for &(id, result, flags) in &completions {
            if !matches!(driver.pending[id], Some(Flight::Receive { .. })) {
                driver.complete_send(id, result, flags);
                continue;
            }
            assert!(
                result > 0,
                "receive failed: {}",
                std::io::Error::from_raw_os_error(-result)
            );
            assert!(!cqueue::more(flags) && !cqueue::notif(flags));
            let Flight::Receive {
                connection,
                mut buffer,
                mut filled,
            } = driver.pending[id].take().unwrap()
            else {
                unreachable!()
            };
            driver.release(id);
            filled += result as usize;
            assert!(filled <= driver.work.size);
            if driver.work.transport == "udp" {
                assert_eq!(filled, driver.work.size);
            }
            if filled < driver.work.size {
                driver.queue(Flight::Receive {
                    connection,
                    buffer,
                    filled,
                });
                continue;
            }
            let offset = driver.storage.offset(&buffer);
            let bytes = &buffer.as_ref()[offset..offset + filled];
            assert_eq!(
                u64::from_le_bytes(bytes[..8].try_into().unwrap()) as usize,
                filled - HEADER
            );
            assert_eq!(
                u64::from_le_bytes(bytes[8..HEADER].try_into().unwrap()) as usize,
                received[connection]
            );
            std::hint::black_box(bytes);
            buffer.truncate(offset + filled);
            let frozen = buffer.freeze();
            if driver.work.retained {
                if let Some(previous) = retained[connection].take() {
                    assert_eq!(
                        u64::from_le_bytes(previous.as_ref()[8..HEADER].try_into().unwrap())
                            as usize
                            + 1,
                        received[connection]
                    );
                }
                retained[connection] = Some(frozen.view(offset..offset + HEADER));
            }
            let sending = Sending {
                connection,
                buffer: frozen.view(offset + HEADER..offset + filled),
            };
            drop(frozen);
            if let Some(output) = &output {
                output.send(sending);
            } else {
                driver.queue(Flight::Send(sending));
            }
            received[connection] += 1;
            if received[connection] < driver.work.iterations {
                waiting.push_back(connection);
            }
        }
    }
    assert!(
        received
            .iter()
            .all(|&count| count == driver.work.iterations)
    );
    driver.unregister();
    std::mem::take(&mut driver.counters)
}

fn peers(work: &Workload) -> (Arc<Vec<OwnedFd>>, Vec<JoinHandle<()>>) {
    let mut sockets = Vec::new();
    let mut peers = Vec::new();
    let listener = work
        .listen
        .as_ref()
        .filter(|_| work.transport == "tcp")
        .map(|address| TcpListener::bind(address).unwrap());
    if let Some(listener) = &listener {
        println!(
            "{}",
            serde_json::json!({"ready":listener.local_addr().unwrap().to_string()})
        );
        std::io::stdout().flush().unwrap();
    }
    for _ in 0..work.depth {
        if let Some(listener) = &listener {
            let (server, _) = listener.accept().unwrap();
            server.set_nodelay(true).unwrap();
            sockets.push(server.into());
            continue;
        }
        if let Some(address) = &work.listen {
            assert_eq!(work.depth, 1, "remote UDP uses one connected socket");
            let server = UdpSocket::bind(address).unwrap();
            println!(
                "{}",
                serde_json::json!({"ready":server.local_addr().unwrap().to_string()})
            );
            std::io::stdout().flush().unwrap();
            let mut hello = [0];
            let (count, peer) = server.recv_from(&mut hello).unwrap();
            assert_eq!(count, 1);
            server.connect(peer).unwrap();
            sockets.push(server.into());
            continue;
        }
        let (server, mut client): (OwnedFd, Box<dyn Peer>) = if work.transport == "tcp" {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let client = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
            let (server, _) = listener.accept().unwrap();
            client.set_nodelay(true).unwrap();
            server.set_nodelay(true).unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(30)))
                .unwrap();
            (server.into(), Box::new(client))
        } else {
            let client = UdpSocket::bind("127.0.0.1:0").unwrap();
            let server = UdpSocket::bind("127.0.0.1:0").unwrap();
            client.connect(server.local_addr().unwrap()).unwrap();
            server.connect(client.local_addr().unwrap()).unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(30)))
                .unwrap();
            (server.into(), Box::new(client))
        };
        sockets.push(server);
        let work = work.clone();
        peers.push(std::thread::spawn(move || client_loop(&work, &mut *client)));
    }
    (Arc::new(sockets), peers)
}
fn client_loop(work: &Workload, client: &mut dyn Peer) {
    let mut request = vec![42; work.size];
    request[..8].copy_from_slice(&((work.size - HEADER) as u64).to_le_bytes());
    let mut response = vec![0; work.size - HEADER];
    for iteration in 0..work.iterations {
        request[8..HEADER].copy_from_slice(&(iteration as u64).to_le_bytes());
        request[HEADER] = iteration as u8;
        request[work.size - 1] = (iteration >> 8) as u8;
        client.exchange(&request, &mut response);
        assert_eq!(response, request[HEADER..]);
    }
}

trait Peer: Send {
    fn exchange(&mut self, request: &[u8], response: &mut [u8]);
}
impl Peer for std::net::TcpStream {
    fn exchange(&mut self, request: &[u8], response: &mut [u8]) {
        self.write_all(request).unwrap();
        self.read_exact(response).unwrap();
    }
}
impl Peer for UdpSocket {
    fn exchange(&mut self, request: &[u8], response: &mut [u8]) {
        assert_eq!(self.send(request).unwrap(), request.len());
        assert_eq!(self.recv(response).unwrap(), response.len());
    }
}

pub(super) fn execute(args: &[String]) -> serde_json::Value {
    if args.get(1).map(String::as_str) == Some("client") {
        return client(args);
    }
    let mode = args.get(1).cloned().unwrap_or("fixed-zc".into());
    let transport = args.get(2).cloned().unwrap_or("tcp".into());
    let size = args.get(3).map(|s| s.parse().unwrap()).unwrap_or(65536);
    let depth = args.get(4).map(|s| s.parse().unwrap()).unwrap_or(1);
    let iterations = args.get(5).map(|s| s.parse().unwrap()).unwrap_or(1000);
    let ownership = args.get(6).map(String::as_str).unwrap_or("retained");
    let placement = args.get(7).map(String::as_str).unwrap_or("local");
    assert!(matches!(
        mode.as_str(),
        "copy" | "registered-copy" | "zc" | "fixed-zc"
    ));
    assert!(matches!(transport.as_str(), "tcp" | "udp"));
    assert!(matches!(ownership, "pooled" | "retained"));
    assert!(matches!(placement, "local" | "handoff"));
    assert!(size > HEADER + 1 && size <= 131072 && depth > 0 && depth <= 8 && iterations > 0);
    if transport == "udp" {
        assert!(size <= 65507);
    }
    let work = Workload {
        mode: mode.clone(),
        transport: transport.clone(),
        size,
        depth,
        iterations,
        retained: ownership == "retained",
        handoff: placement == "handoff",
        listen: args.get(8).cloned(),
    };
    let storage = Storage::new(&work);
    let (sockets, peers) = peers(&work);
    let receiver = Driver::new(work.clone(), storage.clone(), sockets.clone());
    let setup = Instant::now();
    if work.registered() {
        storage.register(&receiver.ring, size);
    }
    let (output, sender) = if work.handoff {
        let sender = Driver::new(work.clone(), storage.clone(), sockets);
        if work.registered() {
            sender
                .ring
                .submitter()
                .register_buffers_clone(receiver.ring.as_raw_fd())
                .unwrap();
        }
        let (channel, input) = mpsc::sync_channel(depth * 2);
        // SAFETY: eventfd returns a new owned descriptor or a negative error, checked before constructing OwnedFd.
        let raw = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC) };
        assert!(raw >= 0);
        let interrupt = Arc::new(unsafe { OwnedFd::from_raw_fd(raw) });
        let output = Output {
            channel: Some(channel),
            interrupt: interrupt.clone(),
        };
        (
            Some(output),
            Some(std::thread::spawn(move || sender.sender(input, interrupt))),
        )
    } else {
        (None, None)
    };
    let setup_seconds = setup.elapsed().as_secs_f64();
    let cpu_started = cpu_time();
    let started = Instant::now();
    let mut counters = receive(receiver, output);
    if let Some(sender) = sender {
        counters.combine(sender.join().unwrap());
    }
    let seconds = started.elapsed().as_secs_f64();
    let cpu_finished = cpu_time();
    for peer in peers {
        peer.join().unwrap();
    }
    let leases: Vec<_> = (0..storage.pool.capacity())
        .map(|_| {
            storage
                .pool
                .try_acquire()
                .expect("all kernel and parser owners retired")
        })
        .collect();
    assert!(storage.pool.try_acquire().is_none());
    drop(leases);
    let bytes = (size - HEADER) * depth * iterations;
    serde_json::json!({"mode":mode,"transport":transport,"size":size,"depth":depth,"iterations":iterations,"ownership":ownership,"placement":placement,"seconds":seconds,"setup_seconds":setup_seconds,"cpu_user_seconds":cpu_finished.0-cpu_started.0,"cpu_system_seconds":cpu_finished.1-cpu_started.1,"bytes":bytes,"gib_per_second":bytes as f64/seconds/1073741824.0,"messages_per_second":(depth*iterations) as f64/seconds,"enters":counters.enters,"cqes":counters.cqes,"sends":counters.sends,"notifications":counters.notifications,"copied_notifications":counters.copied_notifications,"copied_bytes":counters.copied_bytes,"short_sends":counters.short_sends,"pool_stalls":counters.pool_stalls})
}

fn cpu_time() -> (f64, f64) {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    // SAFETY: getrusage initializes this complete rusage on success; no fields are read before checking its result.
    assert_eq!(
        unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) },
        0
    );
    let usage = unsafe { usage.assume_init() };
    let seconds = |time: libc::timeval| time.tv_sec as f64 + time.tv_usec as f64 / 1000000.0;
    (seconds(usage.ru_utime), seconds(usage.ru_stime))
}

fn client(args: &[String]) -> serde_json::Value {
    let transport = args[2].clone();
    let address = args[3].clone();
    let size = args[4].parse().unwrap();
    let depth = args[5].parse().unwrap();
    let iterations = args[6].parse().unwrap();
    assert!(matches!(transport.as_str(), "tcp" | "udp"));
    assert!(size > HEADER + 1 && size <= 131072 && depth > 0 && depth <= 8 && iterations > 0);
    if transport == "udp" {
        assert!(size <= 65507);
    }
    let work = Workload {
        mode: "copy".into(),
        transport: transport.clone(),
        size,
        depth,
        iterations,
        retained: false,
        handoff: false,
        listen: None,
    };
    let mut handles = Vec::new();
    for _ in 0..depth {
        let mut peer: Box<dyn Peer> = if transport == "tcp" {
            let stream = std::net::TcpStream::connect(&address).unwrap();
            stream.set_nodelay(true).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(30)))
                .unwrap();
            Box::new(stream)
        } else {
            let socket = UdpSocket::bind("0.0.0.0:0").unwrap();
            socket.connect(&address).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(30)))
                .unwrap();
            assert_eq!(socket.send(&[0]).unwrap(), 1);
            Box::new(socket)
        };
        let work = work.clone();
        handles.push(std::thread::spawn(move || client_loop(&work, &mut *peer)));
    }
    for handle in handles {
        handle.join().unwrap();
    }
    serde_json::json!({"client_verified":true,"transport":transport,"size":size,"depth":depth,"iterations":iterations})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pipelines_preserve_slices_and_return_every_lease() {
        for mode in ["copy", "registered-copy", "zc", "fixed-zc"] {
            for transport in ["tcp", "udp"] {
                for ownership in ["pooled", "retained"] {
                    for placement in ["local", "handoff"] {
                        let args = [
                            "pipeline", mode, transport, "4096", "2", "4", ownership, placement,
                        ]
                        .map(str::to_owned);
                        let result = execute(&args);
                        assert_eq!(result["bytes"], (4096 - HEADER) * 8);
                    }
                }
            }
        }
    }
    #[test]
    fn large_streams_and_small_packets_preserve_sequence() {
        for mode in ["copy", "fixed-zc"] {
            for (transport, size) in [("tcp", "131072"), ("udp", "1200")] {
                let args = [
                    "pipeline", mode, transport, size, "2", "8", "retained", "handoff",
                ]
                .map(str::to_owned);
                assert_eq!(execute(&args)["iterations"], 8);
            }
        }
    }
    #[test]
    fn short_sends_keep_each_notification_owner_until_retirement() {
        for mode in ["copy", "zc", "fixed-zc"] {
            let work = Workload {
                mode: mode.into(),
                transport: "tcp".into(),
                size: 131072,
                depth: 1,
                iterations: 16,
                retained: true,
                handoff: false,
                listen: None,
            };
            let storage = Storage::new(&work);
            let (sockets, peers) = super::peers(&work);
            let size: libc::c_int = 4096;
            // SAFETY: The live socket and initialized integer have the sizes required by SO_SNDBUF. Limiting its send queue induces valid short stream transfers.
            assert_eq!(
                unsafe {
                    libc::setsockopt(
                        sockets[0].as_raw_fd(),
                        libc::SOL_SOCKET,
                        libc::SO_SNDBUF,
                        (&size as *const libc::c_int).cast(),
                        std::mem::size_of_val(&size) as libc::socklen_t,
                    )
                },
                0
            );
            let driver = Driver::new(work.clone(), storage.clone(), sockets);
            if work.registered() {
                storage.register(&driver.ring, work.size);
            }
            let counters = receive(driver, None);
            for peer in peers {
                peer.join().unwrap();
            }
            assert!(counters.short_sends > 0);
            let leases: Vec<_> = (0..storage.pool.capacity())
                .map(|_| storage.pool.try_acquire().unwrap())
                .collect();
            assert!(storage.pool.try_acquire().is_none());
            drop(leases);
        }
    }
    #[test]
    fn abandoned_receive_does_not_recycle_kernel_storage() {
        let work = Workload {
            mode: "copy".into(),
            transport: "tcp".into(),
            size: 4096,
            depth: 1,
            iterations: 1,
            retained: false,
            handoff: false,
            listen: None,
        };
        let storage = Storage::new(&work);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, _) = listener.accept().unwrap();
        let mut driver = Driver::new(work, storage.clone(), Arc::new(vec![server.into()]));
        driver.queue(Flight::Receive {
            connection: 0,
            buffer: storage.pool.try_acquire().unwrap(),
            filled: 0,
        });
        driver.ring.submit().unwrap();
        drop(driver);
        drop(client);
        // The failure policy deliberately leaks this accepted request's owner. Even after the ring is destroyed, that slot must never be advertised for writing without observing retirement.
        let leases: Vec<_> = (0..storage.pool.capacity() - 1)
            .map(|_| storage.pool.try_acquire().unwrap())
            .collect();
        assert!(storage.pool.try_acquire().is_none());
        drop(leases);
    }

    #[test]
    fn defaults_run_and_invalid_workloads_fail_before_submission() {
        assert_eq!(execute(&["pipeline".into()])["iterations"], 1000);
        let valid = [
            "pipeline", "copy", "tcp", "4096", "1", "1", "pooled", "local",
        ];
        for (position, value) in [
            (1, "invalid"),
            (2, "invalid"),
            (3, "16"),
            (4, "0"),
            (5, "0"),
            (6, "invalid"),
            (7, "invalid"),
        ] {
            let mut args = valid.map(str::to_owned);
            args[position] = value.into();
            assert!(std::panic::catch_unwind(|| execute(&args)).is_err());
        }
        let args = ["pipeline", "copy", "udp", "65536", "1", "1"].map(str::to_owned);
        assert!(std::panic::catch_unwind(|| execute(&args)).is_err());
    }
    fn isolated(work: &Workload) -> (Driver, std::net::TcpStream) {
        let storage = Storage::new(work);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, _) = listener.accept().unwrap();
        (
            Driver::new(work.clone(), storage, Arc::new(vec![server.into()])),
            client,
        )
    }

    #[test]
    fn notifications_decode_actual_zero_copy_and_partial_copy_reports() {
        let work = Workload {
            mode: "zc".into(),
            transport: "tcp".into(),
            size: 4096,
            depth: 1,
            iterations: 1,
            retained: false,
            handoff: false,
            listen: None,
        };
        for result in [0, 128] {
            let (mut driver, _client) = isolated(&work);
            let owner = driver
                .storage
                .pool
                .try_acquire()
                .unwrap()
                .freeze()
                .slice(0..128);
            let id = driver.free.pop().unwrap();
            // Model the valid state after a send result promised a later notification. The kernel has retired its access before this CQE is decoded. Physical-network measurements observe res=0, while newer kernels may report a positive copied-byte count.
            driver.pending[id] = Some(Flight::Notification(owner));
            driver.active = 1;
            driver.complete_send(id, result, 1 << 3);
            assert_eq!(driver.counters.notifications, 1);
            assert_eq!(driver.counters.copied_notifications, 0);
            assert_eq!(driver.counters.copied_bytes, result as u64);
            assert_eq!(driver.active, 0);
        }
    }

    #[test]
    fn truncated_requests_and_reset_peers_fail_without_recycling_live_owners() {
        let work = Workload {
            mode: "copy".into(),
            transport: "tcp".into(),
            size: 4096,
            depth: 1,
            iterations: 1,
            retained: false,
            handoff: false,
            listen: None,
        };
        let (driver, mut client) = isolated(&work);
        client.write_all(&[42; 8]).unwrap();
        client.shutdown(std::net::Shutdown::Write).unwrap();
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| receive(driver, None)))
                .is_err()
        );

        let (mut driver, client) = isolated(&work);
        let linger = libc::linger {
            l_onoff: 1,
            l_linger: 0,
        };
        // SAFETY: SO_LINGER receives the live socket and an initialized native linger value of the required size. Closing with a zero timeout resets this real TCP connection.
        assert_eq!(
            unsafe {
                libc::setsockopt(
                    client.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_LINGER,
                    (&linger as *const libc::linger).cast(),
                    std::mem::size_of_val(&linger) as libc::socklen_t,
                )
            },
            0
        );
        drop(client);
        let buffer = driver.storage.pool.try_acquire().unwrap();
        let offset = driver.storage.offset(&buffer);
        let owner = buffer.freeze().slice(offset..offset + work.size);
        driver.queue(Flight::Send(Sending {
            connection: 0,
            buffer: owner,
        }));
        let mut completions = Vec::new();
        driver.collect(&mut completions);
        let (id, result, flags) = completions.pop().unwrap();
        assert!(completions.is_empty());
        assert!(result < 0);
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                || driver.complete_send(id, result, flags)
            ))
            .is_err()
        );
    }

    #[test]
    fn shared_pool_pressure_drives_completions_and_waits_for_external_owners() {
        let work = Workload {
            mode: "copy".into(),
            transport: "tcp".into(),
            size: 4096,
            depth: 2,
            iterations: 2,
            retained: false,
            handoff: false,
            listen: None,
        };
        let storage = Storage::new(&work);
        let held: Vec<_> = (0..storage.pool.capacity() - 1)
            .map(|_| storage.pool.try_acquire().unwrap())
            .collect();
        let (sockets, peers) = super::peers(&work);
        let counters = receive(Driver::new(work.clone(), storage.clone(), sockets), None);
        assert!(counters.pool_stalls > 0);
        for peer in peers {
            peer.join().unwrap();
        }
        drop(held);

        let held: Vec<_> = (0..storage.pool.capacity())
            .map(|_| storage.pool.try_acquire().unwrap())
            .collect();
        let (sockets, peers) = super::peers(&work);
        let returning = std::thread::spawn(move || {
            // Application processing may retain all capacity independently of either ring. Release it later while the receive thread waits without an outstanding kernel request.
            std::thread::sleep(Duration::from_millis(50));
            drop(held);
        });
        receive(Driver::new(work, storage, sockets), None);
        returning.join().unwrap();
        for peer in peers {
            peer.join().unwrap();
        }
    }
}
