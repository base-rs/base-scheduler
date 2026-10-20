from pathlib import Path
import subprocess
root=Path('/home/samuel/Developer/core-rs/core-scheduler-perf-tokio-next');out=Path('/home/samuel/.cache/core-scheduler-tokio-next-results/counters')
p=root/'crates/tokio/src/tokio_socket.rs';s=p.read_text();(out/'tokio_socket.plain.rs').write_text(s)
s=s.replace('''        let result = operation(self.get_ref());
''','''        let result = operation(self.get_ref());
        counters::record(interest == Interest::Readable, false, &result);
''')
s=s.replace('''                    .async_io(interest, |_| operation(self.inner.get_ref()))
''','''                    .async_io(interest, |_| {
                        let result = operation(self.inner.get_ref());
                        counters::record(interest == tokio::io::Interest::READABLE, true, &result);
                        result
                    })
''')
s=s.replace('''                    if let Ok(result) = ready.try_io(|_| operation(self.inner.get_ref())) {
''','''                    if let Ok(result) = ready.try_io(|_| {
                        let result = operation(self.inner.get_ref());
                        counters::record(interest == tokio::io::Interest::READABLE, true, &result);
                        result
                    }) {
''')
s+='''
// Experiment-only counters. Excluded from timing builds and production changes.
mod counters {
    use std::{cell::Cell, io};
    thread_local! { static COUNTERS: Counters = const { Counters([const { Cell::new(0) }; 8]) }; }
    struct Counters([Cell<u64>; 8]);
    pub(super) fn record<T>(read: bool, retry: bool, result: &io::Result<T>) {
        let ok = match result { Ok(_) => true, Err(e) if e.kind() == io::ErrorKind::WouldBlock => false, Err(_) => return };
        let index = (if read { 0 } else { 4 }) + (if retry { 2 } else { 0 }) + (if ok { 0 } else { 1 });
        COUNTERS.with(|c| c.0[index].set(c.0[index].get()+1));
    }
    impl Drop for Counters {
        fn drop(&mut self) {
            if self.0.iter().all(|x| x.get()==0) { return; }
            let x = self.0.each_ref().map(|x| x.get());
            eprintln!("SOCKET_COUNTERS {{\\\"first_read_ok\\\":{},\\\"first_read_wouldblock\\\":{},\\\"retry_read_ok\\\":{},\\\"retry_read_wouldblock\\\":{},\\\"first_write_ok\\\":{},\\\"first_write_wouldblock\\\":{},\\\"retry_write_ok\\\":{},\\\"retry_write_wouldblock\\\":{}}}", x[0],x[1],x[2],x[3],x[4],x[5],x[6],x[7]);
        }
    }
}
'''
p.write_text(s);(out/'source.patch').write_bytes(subprocess.check_output(['git','diff'],cwd=root))
