#[cfg(test)]
mod tests {
    use std::cell::UnsafeCell;

    struct Scalar<B> {
        buffer: UnsafeCell<B>,
        pointer: usize,
    }
    impl<B> Scalar<B> {
        fn entry(&self) -> *mut u8 {
            self.pointer as *mut u8
        }
    }

    fn scalar<B: AsRef<[u8]> + AsMut<[u8]>>(buffer: B) {
        let mut request = Box::new(Scalar { buffer: UnsafeCell::new(buffer), pointer: 0 });
        request.pointer = request.buffer.get_mut().as_mut().as_mut_ptr() as usize;
        let pending: Box<dyn Prepared> = request;
        let pointer = pending.entry();
        // Simulate an original kernel completion after shared entry/bookkeeping.
        unsafe { pointer.write(42) };
        assert_eq!(pending.completed(), 42);
    }
    trait Prepared {
        fn entry(&self) -> *mut u8;
        fn completed(self: Box<Self>) -> u8;
    }
    impl<B: AsRef<[u8]>> Prepared for Scalar<B> {
        fn entry(&self) -> *mut u8 { self.entry() }
        fn completed(self: Box<Self>) -> u8 { self.buffer.into_inner().as_ref()[0] }
    }

    #[test]
    fn scalar_box() { scalar(vec![0; 4].into_boxed_slice()); }
    #[test]
    fn scalar_vec() { scalar(vec![0; 4]); }

    #[derive(Clone, Copy)]
    struct Iovec { base: *mut u8, length: usize }
    struct Buffers { bytes: Box<[u8]>, vectors: [Iovec; 1] }
    struct Header { vectors: *mut Iovec }
    struct Message { buffers: UnsafeCell<Buffers>, header: UnsafeCell<Header> }
    impl Message {
        fn prepare(&mut self) {
            let buffers = self.buffers.get_mut();
            buffers.vectors[0].base = buffers.bytes.as_mut_ptr();
            self.header.get_mut().vectors = buffers.vectors.as_mut_ptr();
        }
        fn entry(&self) -> *mut Header { self.header.get() }
    }
    #[test]
    fn inline_message_vectors_remain_valid_across_shared_entry() {
        let mut request = Box::new(Message {
            buffers: UnsafeCell::new(Buffers {
                bytes: vec![0; 4].into_boxed_slice(),
                vectors: [Iovec { base: std::ptr::null_mut(), length: 4 }],
            }),
            header: UnsafeCell::new(Header { vectors: std::ptr::null_mut() }),
        });
        request.prepare();
        let header = request.entry();
        // Simulate kernel traversing the immutable native header/vector package.
        unsafe {
            let vector = (*header).vectors.read();
            assert_eq!(vector.length, 4);
            vector.base.write(42);
        }
        assert_eq!(request.buffers.into_inner().bytes[0], 42);
    }
}
