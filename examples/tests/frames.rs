// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use base_scheduler::*;
use base_scheduler_examples::frames::{Frame, FrameDecoder};

#[test]
fn retained_ranges_prevent_reuse_and_fragmented_records_copy_only_their_bytes() {
    let pool = HeapBufferPool::new(2, 16).unwrap();
    let mut first = pool.try_acquire().unwrap();
    first.as_mut()[..14].copy_from_slice(b"aaaabbbbccccdd");
    first.truncate(14);
    let address = first.as_ref().as_ptr();
    let mut decoder = FrameDecoder::new(4).unwrap();
    let mut frames = Vec::new();
    decoder
        .push(first.freeze(), |frame| {
            frames.push(frame);
            Ok(())
        })
        .unwrap();
    assert_eq!(frames.len(), 3);
    assert_eq!(frames[0].as_ref().as_ptr(), address);
    assert!(matches!(frames[0], Frame::Shared(_)));
    let mut second = pool.try_acquire().unwrap();
    second.as_mut()[..10].copy_from_slice(b"ddeeeeffff");
    second.truncate(10);
    decoder
        .push(second.freeze(), |frame| {
            frames.push(frame);
            Ok(())
        })
        .unwrap();
    decoder.finish().unwrap();
    assert!(pool.try_acquire().is_none());
    assert!(matches!(frames[3], Frame::Copied(_)));
    let expected = [b"aaaa", b"bbbb", b"cccc", b"dddd", b"eeee", b"ffff"];
    for (frame, expected) in frames.iter().zip(expected) {
        assert_eq!(frame.as_ref(), expected);
    }
    drop(frames);
    let first = pool.try_acquire().unwrap();
    let second = pool.try_acquire().unwrap();
    drop((first, second));
}

#[test]
fn framing_handles_tiny_chunks_incomplete_eof_and_consumer_errors() {
    assert!(FrameDecoder::new(0).is_err());
    let pool = HeapBufferPool::new(1, 1).unwrap();
    let mut decoder = FrameDecoder::new(4).unwrap();
    for byte in b"abc" {
        let mut buffer = pool.try_acquire().unwrap();
        buffer.as_mut()[0] = *byte;
        decoder.push(buffer.freeze(), |_| unreachable!()).unwrap();
    }
    assert_eq!(
        decoder.finish().unwrap_err().kind(),
        std::io::ErrorKind::UnexpectedEof
    );
    let mut buffer = pool.try_acquire().unwrap();
    buffer.as_mut()[0] = b'd';
    let error = decoder
        .push(buffer.freeze(), |frame| {
            assert_eq!(frame.as_ref(), b"abcd");
            Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe).into())
        })
        .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);
    let mut decoder = FrameDecoder::new(1).unwrap();
    assert!(
        decoder
            .push(pool.try_acquire().unwrap().freeze(), |_| Err(
                std::io::Error::from(std::io::ErrorKind::BrokenPipe).into()
            ))
            .is_err()
    );
}
