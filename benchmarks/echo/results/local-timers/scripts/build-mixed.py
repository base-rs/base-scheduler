import fcntl,hashlib,json,shutil,subprocess
from pathlib import Path
root=Path.home()/'.cache/core-scheduler-local-timers-final';checkout=Path.home()/'Developer/core-rs/core-scheduler-local-timers'
source=checkout/'benchmarks/echo/src/server.rs'
start='''            let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let start = std::time::Instant::now() + std::time::Duration::from_secs(1);
            let count: usize = std::env::var("CORE_TIMER_TASKS").unwrap().parse().unwrap();
            let mut timers = Vec::new();
            for _ in 0..count {
                let shared = Arc::clone(&scheduler);
                let stop = Arc::clone(&stop);
                timers.push(scheduler.spawn(async move {
                    use core_scheduler::Clock;
                    let never = Cancellation::never();
                    let mut samples = hdrhistogram::Histogram::<u64>::new_with_bounds(1, 60_000_000_000, 3).unwrap();
                    while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                        let target = std::time::Instant::now() + std::time::Duration::from_millis(1);
                        shared.sleep_until(target, &never).await.unwrap();
                        let now = std::time::Instant::now();
                        assert!(now >= target);
                        if target >= start && !stop.load(std::sync::atomic::Ordering::Relaxed) {
                            samples.record(now.duration_since(target).as_nanos().max(1) as u64).unwrap();
                        }
                    }
                    samples
                }).unwrap());
            }
'''
end='''            stop.store(true, std::sync::atomic::Ordering::Relaxed);
            let mut samples = hdrhistogram::Histogram::<u64>::new_with_bounds(1, 60_000_000_000, 3).unwrap();
            for timer in timers { samples.add(scheduler.block_on(timer).unwrap()).unwrap(); }
            eprintln!("{}", serde_json::json!({"tasks":count,"completed":samples.len(),"lateness_p50_us":samples.value_at_quantile(0.5) as f64/1000.0,"lateness_p99_us":samples.value_at_quantile(0.99) as f64/1000.0,"seconds_after_warmup":start.elapsed().as_secs_f64()}));
'''
with open('/tmp/core-scheduler-perf.lock','a') as lock:
 print('Waiting for mixed build lock',flush=True);fcntl.flock(lock,fcntl.LOCK_EX)
 original=source.read_text()
 needle='            ready(&listener);\n            scheduler.block_on(scheduler_server('
 assert original.count(needle)==1
 mixed=original.replace(needle,start+needle).replace('            scheduler.block_on(scheduler.shutdown());',end+'            scheduler.block_on(scheduler.shutdown());')
 source.write_text(mixed);(root/'mixed-server.rs').write_text(mixed)
 report={'harness_sha256':hashlib.sha256(mixed.encode()).hexdigest(),'binaries':{}}
 try:
  for variant in ['before','local']:
   if variant=='before':subprocess.run(['git','apply','-R',str(root/'candidate.patch')],cwd=checkout,check=True)
   else:subprocess.run(['git','apply',str(root/'candidate.patch')],cwd=checkout,check=True)
   command=['cargo','build','--release','--locked','-p','core-scheduler-echo-benchmark','--bin','core-scheduler-echo-benchmark']
   if variant=='local':command+=['--features','core-scheduler-native/worker-timers']
   with (root/(variant+'-mixed-build.log')).open('w') as output:subprocess.run(command,cwd=checkout,stdout=output,stderr=subprocess.STDOUT,check=True)
   dest=root/(variant+'-mixed');shutil.copy2(checkout/'target/release/core-scheduler-echo-benchmark',dest)
   report['binaries'][variant]={'path':str(dest),'sha256':hashlib.sha256(dest.read_bytes()).hexdigest()}
   print('Built mixed',variant,flush=True)
 finally:source.write_text(original)
 (root/'mixed-build.json').write_text(json.dumps(report,indent=2)+'\n')
