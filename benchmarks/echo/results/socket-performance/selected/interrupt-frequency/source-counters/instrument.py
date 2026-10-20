from pathlib import Path
import sys
root=Path(sys.argv[1]); path=root/'crates/iouring/src/io_uring_selector.rs'; s=path.read_text()
fields=['notify_local_attempt','notify_local_success','notify_local_eagain','notify_local_error','notify_remote_attempt','notify_remote_success','notify_remote_eagain','notify_remote_error','wait_function','wait_syscall','submit_syscall','notification_armed','notification_cqe','notification_cqe_success','notification_cqe_error','sync_read_attempt','sync_read_success','sync_read_eagain','sync_read_error','payload_cqe','turn']
module='''// Experimental worker-local counters; deliberately excluded from shipping code.
use std::{cell::Cell, io};
thread_local! { static COUNTERS: Counters = const { Counters::new() }; }
struct Counters { active_worker: Cell<bool>, ever_worker: Cell<bool>, values: [Cell<u64>; '''+str(len(fields))+'''] }
impl Counters {
    const fn new() -> Self { Self { active_worker: Cell::new(false), ever_worker: Cell::new(false), values: [const { Cell::new(0) }; '''+str(len(fields))+'''] } }
    #[inline(always)] fn add(&self, index: usize) { let value=&self.values[index]; value.set(value.get()+1); }
}
'''
for i,f in enumerate(fields):module+='pub(crate) const '+f.upper()+': usize = '+str(i)+';\n'
module+='''#[inline(always)] pub(crate) fn increment(index:usize) { COUNTERS.with(|c| c.add(index)); }
pub(crate) fn worker(active: bool) { COUNTERS.with(|c| { c.active_worker.set(active); if active { c.ever_worker.set(true); } }); }
#[inline(always)] pub(crate) fn syscall<T>(name: &'static str, result: &io::Result<T>) {
    match name {
        "notify" => COUNTERS.with(|c| {
            let first=if c.active_worker.get() { NOTIFY_LOCAL_ATTEMPT } else { NOTIFY_REMOTE_ATTEMPT };
            c.add(first);
            c.add(first+match result { Ok(_) => 1, Err(e) if e.kind()==io::ErrorKind::WouldBlock => 2, Err(_) => 3 });
        }),
        "wait" => increment(WAIT_SYSCALL),
        "submit" => increment(SUBMIT_SYSCALL),
        "notification-read" => COUNTERS.with(|c| {
            c.add(SYNC_READ_ATTEMPT);
            c.add(match result { Ok(_) => SYNC_READ_SUCCESS, Err(e) if e.kind()==io::ErrorKind::WouldBlock => SYNC_READ_EAGAIN, Err(_) => SYNC_READ_ERROR });
        }),
        "notification-completion" => COUNTERS.with(|c| { c.add(if result.is_ok() { NOTIFICATION_CQE_SUCCESS } else { NOTIFICATION_CQE_ERROR }); }),
        _ => (),
    }
}
impl Drop for Counters {
    fn drop(&mut self) {
        use std::fmt::Write;
        if self.values.iter().all(|v| v.get()==0) { return; }
        let mut output=String::new();
        write!(&mut output,"{{\\\"worker\\\":{}",self.ever_worker.get()).unwrap();
'''
for i,f in enumerate(fields):module+='        write!(&mut output,",\\\"'+f+'\\\":{}",self.values['+str(i)+'].get()).unwrap();\n'
module+='''        output.push('}');
        eprintln!("INTERRUPT_COUNTERS {}",output);
    }
}
'''
(root/'crates/iouring/src/interrupt_counters.rs').write_text(module)
s=s.replace('const CANCEL_TAG: u64 = 1 << 63;', '#[path = "interrupt_counters.rs"]\npub(crate) mod interrupt_counters;\nuse interrupt_counters as counters;\n\nconst CANCEL_TAG: u64 = 1 << 63;',1)
old='''        let _ = name;
        operation()''';new='''        let result = operation();
        counters::syscall(name, &result);
        result''';assert s.count(old)==1;s=s.replace(old,new)
needle='''    pub(crate) fn turn(&mut self, timeout: Option<Duration>) -> io::Result<bool> {
''';assert s.count(needle)==1;s=s.replace(needle,needle+'        counters::increment(counters::TURN);\n')
needle='''            if identifier == NOTIFICATION_TAG {
''';assert s.count(needle)==1;s=s.replace(needle,needle+'                counters::increment(counters::NOTIFICATION_CQE);\n')
needle='''            // Every untagged CQE is the single original completion of a
''';assert s.count(needle)==1;s=s.replace(needle,'            counters::increment(counters::PAYLOAD_CQE);\n'+needle)
needle='''                self.notification_pending = true;
''';assert s.count(needle)==1;s=s.replace(needle,'                counters::increment(counters::NOTIFICATION_ARMED);\n'+needle)
needle='''    fn wait(&mut self, timeout: Option<Duration>) -> io::Result<()> {
''';assert s.count(needle)==1;s=s.replace(needle,needle+'        counters::increment(counters::WAIT_FUNCTION);\n')
path.write_text(s)
p=root/'crates/iouring/src/io_uring_worker.rs';s=p.read_text();needle='''        Self { driver, route }
''';assert s.count(needle)==1;s=s.replace(needle,'        crate::io_uring_selector::interrupt_counters::worker(true);\n'+needle)
needle='''        self.driver.stop();
''';assert s.count(needle)==1;s=s.replace(needle,needle+'        crate::io_uring_selector::interrupt_counters::worker(false);\n');p.write_text(s)
