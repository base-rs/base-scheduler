from pathlib import Path
import json, statistics, hashlib
base=Path('/home/samuel/.cache/core-scheduler-self-wake');d=json.loads((base/'matched/aiko.json').read_text());assert len(d['samples'])==24
labels=['prototype-frozen','prototype-fixed','selected-frozen','selected-fixed','prototype-frozen-counted','prototype-fixed-counted','selected-frozen-counted','selected-fixed-counted']
med=lambda rows,key:statistics.median(key(x) for x in rows)
lines=['# Same-thread worker wake suppression experiment','','The runtime patch retains sleeping.swap(false, SeqCst), publication fences, peer fanout, and remote interrupts. It skips unpark and driver_waker only when the registered target thread is the current thread. No other runtime changes were made.','','Three randomized serial TCP echo samples per executable: one worker, one connection, 64 bytes, 1 s warmup plus 3 s measurement. All server context switches and source counters cover lifecycle including warmup and shutdown. Counter builds and plain timing builds are separate. Builds held shared /tmp/core-scheduler-perf.lock; the matrix held it exclusively.','','| Variant | Median measured req/s | Median lifecycle voluntary CSW | req/s samples |','|---|---:|---:|---|']
for label in labels:
 rows=[x for x in d['samples'] if x['variant']==label]
 lines.append(f"| {label} | {med(rows,lambda x:x['requests_per_second']):,.0f} | {med(rows,lambda x:x['server_cpu']['voluntary_switches']):,.0f} | "+', '.join(f"{x['requests_per_second']:,.0f}" for x in rows)+' |')
lines+=['','Plain fixed/frozen median throughput changes:','']
for family in ['prototype','selected']:
 before=[x for x in d['samples'] if x['variant']==family+'-frozen'];after=[x for x in d['samples'] if x['variant']==family+'-fixed']
 change=100*(med(after,lambda x:x['requests_per_second'])/med(before,lambda x:x['requests_per_second'])-1)
 lines.append(f'- {family}: {change:+.2f}%.')
lines+=['','| Counter variant/repeat | Payload CQEs / 2 | Same-worker notify success | Other-thread notify success | Interrupt SQEs/CQEs | Wait syscalls | Voluntary CSW |','|---|---:|---:|---:|---|---:|---:|']
for label in labels:
 if not label.endswith('counted'):continue
 for x in sorted([x for x in d['samples'] if x['variant']==label],key=lambda x:x['repeat']):
  c=x['counters'];lines.append(f"| {label}/{x['repeat']} | {x['lifecycle_payload_cqe_pairs']:,.1f} | {c['notify_local_success']:,} | {c['notify_remote_success']:,} | {c['notification_armed']:,}/{c['notification_cqe']:,} | {c['wait_syscall']:,} | {x['server_cpu']['voluntary_switches']:,} |")
lines+=['','The counter TLS marker classifies the installed worker thread; in this one-worker case that is the target worker. Payload Submit commands already enter the matching TLS queue and return before notify; Controller::Wake can notify independently. Untagged original CQEs in this workload are socket Recv/Send attempts, including any retries or partial transfers and final EOF; accept, registration and TCP_NODELAY use the readiness fallback. Payload CQEs / 2 estimates lifecycle RTTs. It is not an exact successful measured-request count.','','The final EOF contributes one additional Recv with no matching Send. The odd .5 totals are consistent with this, but these aggregate counters do not independently establish absence of unsuccessful attempts. The selected implementation uses stream notifications and readiness poller, so it has no interrupt SQE/CQE. Its wait_function counter remains unused; wait_syscall is the relevant wait count.','','Source counters prove whether same-worker notification calls were removed. They do not prove each blocking-capable wait actually slept; voluntary context switches are a separate lifecycle observation. Instrumentation can change interleaving even when aggregate throughput appears close.','','Native and io_uring package tests passed (see tests.log), including existing parallel-dispatch coverage. The earlier source-counter experiment and original baseline matrices remain untouched. No commits were pushed.']
(base/'analysis.md').write_text('\n'.join(lines)+'\n')
d['runner_sha256']=hashlib.sha256((base/'run.py').read_bytes()).hexdigest();d['sample_runner_sha256']=hashlib.sha256((base/'sample-runner.py').read_bytes()).hexdigest()
meta={k:v for k,v in d.items() if k!='samples'}
(base/'provenance.json').write_text(json.dumps(meta,indent=2)+'\n')
print('\n'.join(lines[:20]),flush=True)
