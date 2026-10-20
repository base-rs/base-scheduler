from pathlib import Path
import difflib
root=Path('/home/samuel/Developer/core-rs/core-scheduler-perf-tokio')
out=Path('/home/samuel/.cache/core-scheduler-direct-notification-write-fastpath/counted')
p=root/'crates/iouring/src/io_uring_selector.rs';s=p.read_text()
s=s.replace('const CANCEL_TAG: u64 = 1 << 63;', '#[path = "notification_counters.rs"]\npub(crate) mod notification_counters;\nuse notification_counters as counters;\n\nconst CANCEL_TAG: u64 = 1 << 63;',1)
old='''    fn notify(&self) {
''';assert s.count(old)==1;s=s.replace(old,old+'        let owner = crate::io_uring_worker::counter_owns(self);\n')
old='''                if result < 0 {
                    Err(io::Error::last_os_error())
''';start=s.index('    fn notify(&self) {');end=s.index('    pub(crate) fn close(&self)',start);a=s[:start];b=s[start:end];c=s[end:];assert b.count(old)==1
b=b.replace(old,'''                counters::write(owner, result >= 0);
'''+old);s=a+b+c
old='''        if !crate::io_uring_worker::notify(self) {
            self.notify();
        }
''';assert s.count(old)==1;s=s.replace(old,'''        if crate::io_uring_worker::notify(self) {
            counters::increment(counters::OWNER_WAKE_AVOIDED);
        } else {
            self.notify();
        }
''')
old='''            if identifier == NOTIFICATION_TAG {
''';assert s.count(old)==1;s=s.replace(old,old+'                counters::increment(counters::NOTIFICATION_CQE);\n')
old='''            // Every untagged CQE is the single original completion of a
''';assert s.count(old)==1;s=s.replace(old,'            counters::increment(counters::PAYLOAD_CQE);\n'+old)
p.write_text(s)
p=root/'crates/iouring/src/io_uring_worker.rs';s=p.read_text();s+='''
// Experimental exact-controller owner classification; excluded from shipping code.
pub(crate) fn counter_owns(controller: &Controller) -> bool {
    ROUTES.with(|routes| routes.borrow().iter().any(|route| std::ptr::eq(Arc::as_ptr(&route.controller), controller)))
}
''';p.write_text(s)
fields=['owner_write_attempt','remote_write_attempt','owner_write_success','remote_write_success','owner_wake_avoided','payload_cqe','notification_cqe']
m='''// Experimental notification counts; excluded from shipping code and timing binaries.
use std::cell::Cell;
thread_local! { static COUNTERS: Counters = const { Counters { values: [const { Cell::new(0) }; 7] } }; }
struct Counters { values: [Cell<u64>; 7] }
'''
for i,f in enumerate(fields):m+=f'pub(crate) const {f.upper()}: usize = {i};\n'
m+='''pub(crate) fn increment(index: usize) { COUNTERS.with(|c| c.values[index].set(c.values[index].get()+1)); }
pub(crate) fn write(owner: bool, success: bool) {
    increment(if owner { OWNER_WRITE_ATTEMPT } else { REMOTE_WRITE_ATTEMPT });
    if success { increment(if owner { OWNER_WRITE_SUCCESS } else { REMOTE_WRITE_SUCCESS }); }
}
impl Drop for Counters {
    fn drop(&mut self) {
        use std::fmt::Write;
        if self.values.iter().all(|x| x.get()==0) { return; }
        let mut s=String::from("{");
'''
for i,f in enumerate(fields):m+=f'        write!(&mut s, "'+('' if i==0 else ',')+'\\"'+f+'\\":{}", self.values['+str(i)+'].get()).unwrap();\n'
m+='''        s.push('}');
        eprintln!("NOTIFICATION_COUNTERS {}", s);
    }
}
'''
(root/'crates/iouring/src/notification_counters.rs').write_text(m)
patch=''
for file,plain in [('crates/iouring/src/io_uring_selector.rs','selector-plain.rs'),('crates/iouring/src/io_uring_worker.rs','worker-plain.rs')]:
 patch+=''.join(difflib.unified_diff((out/plain).read_text().splitlines(True),(root/file).read_text().splitlines(True),fromfile='a/'+file,tofile='b/'+file))
file='crates/iouring/src/notification_counters.rs';patch+=''.join(difflib.unified_diff([],(root/file).read_text().splitlines(True),fromfile='/dev/null',tofile='b/'+file))
(out/'instrumentation.patch').write_text(patch)
