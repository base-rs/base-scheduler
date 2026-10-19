// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use core_scheduler::{BufferResult, Cancellation, FileOperation, FileRead, IoBufMut, Spawn};
use core_scheduler_native::{NativeExecutorOptions, NativeScheduler, ReadinessSelector, Selector};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

struct MemorySelector {
    closed: Arc<AtomicBool>,
    retired: Arc<AtomicUsize>,
}
impl Selector for MemorySelector {
    fn close(&self) {
        self.closed.store(true, Ordering::Release);
    }
    async fn shutdown(&self) {
        self.close();
        self.retired.fetch_add(1, Ordering::Relaxed);
    }
}
impl FileOperation for MemorySelector {
    type File = [u8; 3];
}
impl FileRead for MemorySelector {
    async fn file_read<B: IoBufMut>(
        &self,
        file: &Self::File,
        mut buffer: B,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        if let Err(error) = cancellation.check() {
            return (Err(error.into()), buffer);
        }
        let count = file.len().min(buffer.as_ref().len());
        buffer.as_mut()[..count].copy_from_slice(&file[..count]);
        (Ok(count), buffer)
    }
}

#[test]
fn a_partial_selector_keeps_its_own_resource_type_and_needs_no_executor() {
    let closed = Arc::new(AtomicBool::new(false));
    let retired = Arc::new(AtomicUsize::new(0));
    let selector = MemorySelector {
        closed: Arc::clone(&closed),
        retired: Arc::clone(&retired),
    };
    let scheduler = NativeScheduler::with_selector(
        selector,
        NativeExecutorOptions {
            workers: 1,
            ..Default::default()
        },
    )
    .unwrap();
    scheduler.block_on(async {
        let buffer = vec![0; 3];
        let pointer = buffer.as_ptr();
        let (result, buffer) = scheduler
            .file_read(b"abc", buffer, &Cancellation::never())
            .await;
        assert_eq!(result.unwrap(), 3);
        assert_eq!(buffer, b"abc");
        assert_eq!(pointer, buffer.as_ptr());
        assert_eq!(scheduler.spawn(async { 42 }).unwrap().await.unwrap(), 42);
        scheduler.shutdown().await;
    });
    assert!(closed.load(Ordering::Acquire));
    assert_eq!(retired.load(Ordering::Relaxed), 1);
}

#[test]
fn failed_composition_closes_the_supplied_selector() {
    let closed = Arc::new(AtomicBool::new(false));
    let selector = MemorySelector {
        closed: Arc::clone(&closed),
        retired: Arc::default(),
    };
    assert!(
        NativeScheduler::with_selector(
            selector,
            NativeExecutorOptions {
                workers: 0,
                ..Default::default()
            }
        )
        .is_err()
    );
    assert!(closed.load(Ordering::Acquire));
}

#[test]
fn readiness_is_usable_without_native_task_workers() {
    futures_lite::future::block_on(async {
        let selector = ReadinessSelector::new().unwrap();
        let (client, server) = core_scheduler_conformance::socket::stream_pair(
            &selector,
            core_scheduler::SocketDomain::Ipv4,
        )
        .await;
        core_scheduler_conformance::socket::stream(&selector, &client, &server).await;
        selector.shutdown().await;
    });
}
