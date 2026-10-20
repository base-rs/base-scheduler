from pathlib import Path
from statistics import median
import json
out=Path(__file__).resolve().parent
root=out.parents[2]
d=json.loads((out/'aiko.json').read_text())
backends=['tokio','tokio-adapter','tokio-adapter-poll','iouring','iouring-poll']
workloads=sorted({(s['workers'],s['connections'],s['bytes']) for s in d['samples']})
assert len(d['samples'])==75 and len(workloads)==5
summary=[]
for workers,connections,size in workloads:
 row={'workers':workers,'connections':connections,'bytes':size,'backends':{}}
 for backend in backends:
  samples=[s for s in d['samples'] if (s['workers'],s['connections'],s['bytes'],s['backend'])==(workers,connections,size,backend)]
  assert {s['repeat'] for s in samples}=={1,2,3} and len(samples)==3
  for s in samples:
   assert json.loads((out/(s['identifier']+'.json')).read_text())==s
  rates=[s['requests_per_second'] for s in samples]
  row['backends'][backend]={'median_rps':median(rates),'min_rps':min(rates),'max_rps':max(rates),'median_p50_us':median(s['rtt_p50_us'] for s in samples),'median_p99_us':median(s['rtt_p99_us'] for s in samples)}
 control=row['backends']['tokio']['median_rps']
 for backend in backends:row['backends'][backend]['pct_of_tokio']=100*row['backends'][backend]['median_rps']/control
 summary.append(row)
(out/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')
lines=['# Current comparison with pure Tokio','','This comparison measures the native owner-wake improvements against pure Tokio and both Tokio adapter paths using the same frozen optimized server executable for all five backends. Production runtime source is `3af77be0f8b8a9303c8680a7f8aab49b1c3d304c`; server SHA-256 is `'+d['server']['sha256']+'`. The [raw report](results/socket-performance/owner-wake-vs-tokio/aiko.json) records its source/build provenance, fixed client, runner hashes, and all samples.','','Aiko runs three randomized repetitions per backend and workload, with one second of warmup and three seconds of measurement. Server CPUs are 0 or 0–3; the four-worker client uses CPUs 4–7. Runs hold the exclusive benchmark lock. These 75 samples are separate from the earlier before/after native comparison and historical Tokio matrices.','','## Throughput relative to pure Tokio','','Percentages are ratios of median complete round trips per second; 100% means equal throughput to pure Tokio in this workload. The owned native path uses completion reads and attempts ready scalar writes synchronously. Its borrowed path uses nonblocking syscalls followed by ring readiness waits. Both Tokio adapter paths use readiness.','','| Workers | Connections | Bytes | Pure Tokio requests/s | Tokio owned | Tokio borrowed | Native owned | Native borrowed |','| --- | --- | --- | --- | --- | --- | --- | --- |']
for row in summary:
 b=row['backends'];pcts=' | '.join(f"{b[k]['pct_of_tokio']:.1f}%" for k in backends[1:])
 lines.append(f"| {row['workers']} | {row['connections']} | {row['bytes']} | {b['tokio']['median_rps']:,.0f} | {pcts} |")
lines+=['','With four workers, the Tokio adapter reaches 94.9–96.0% of pure Tokio throughput and native reaches 93.2–94.5%. With one worker and 32 connections, the adapter reaches 88.2–90.7%, native owned reaches 89.7–90.4%, and native borrowed reaches 80.9–82.3%. The adapter therefore does not consistently meet the 5% target, and native remains behind pure Tokio in every measured workload. The single-connection adapter result is approximately equal to pure Tokio.','','## Absolute throughput and variation','','Minimum–maximum complete round trips per second across three repetitions, in millions. Full-precision medians and latency summaries are in the [summary](results/socket-performance/owner-wake-vs-tokio/summary.json).','','| Workers/connections/bytes | Pure Tokio | Tokio owned | Tokio borrowed | Native owned | Native borrowed |','| --- | --- | --- | --- | --- | --- |']
for row in summary:
 cells=' | '.join(f"{row['backends'][k]['min_rps']/1e6:.3f}–{row['backends'][k]['max_rps']/1e6:.3f}" for k in backends)
 lines.append(f"| {row['workers']}/{row['connections']}/{row['bytes']} | {cells} |")
lines+=['','This is a loopback TCP echo workload with persistent connections and one outstanding request per connection. It does not establish general application performance or physical-network throughput. Percentages describe these concrete implementations and buffer paths, including scheduling and I/O, rather than isolating trait-dispatch cost. Three repetitions give a focused comparison rather than a guarantee about every run. Raw samples retain measured-window latency and lifecycle CPU use; no sample was removed or reconstructed.','','The [owner-wake report](self-interrupts.md) describes the implementation and separate native before/after measurements. The [historical Tokio report](socket-performance.md) remains unchanged as a measurement of its earlier runtime.']
(root/'tokio-comparison.md').write_text('\n'.join(lines)+'\n')
print('\n'.join(lines[8:20]))
