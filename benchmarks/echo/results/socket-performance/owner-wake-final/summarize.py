from pathlib import Path
import hashlib,json,statistics
p=Path('/home/samuel/.cache/core-scheduler-owner-wake-final')
source=json.loads((p/'source-commit.json').read_text());prov=json.loads((p/'provenance.json').read_text());prov.update(source)
lines=['# Final native owner-wake benchmark','','Production source: `'+source['production_source_commit']+'`; verified runtime patch SHA-256 `'+source['production_patch_sha256']+'`. The benchmark worktree applies that exact production patch to `63da7551abbd777ac0f746ae2530eb7968009fe4`. Additional test/docs files from the source commit do not affect this executable.','','Optimized release builds use `cargo build --release --locked -p core-scheduler-echo-benchmark`, with no release-profile or RUSTFLAGS overrides. Original control and final candidate use the same build configuration and fixed client. Each case has three randomized repetitions, one-second warmup and three-second measurement; server CPUs 0 or 0–3 and client CPUs 4–7. All timing/idle runs hold the exclusive benchmark lock. These results compare the final native backend against its original native baseline, not against pure Tokio.','']
for phase,expected in [('concurrent',48),('serial64',12)]:
 report=json.loads((p/phase/'aiko.json').read_text());assert len(report['samples'])==expected
 assert len(list((p/phase).glob('*-r*.json')))==expected
 report['provenance'].update(source)
 (p/phase/'aiko.json').write_text(json.dumps(report,indent=2)+'\n')
 lines+=['## '+('Concurrent throughput' if phase=='concurrent' else 'Serial 64-byte throughput'),'','| Workers | Connections | Bytes | Backend | Original requests/s | Final requests/s | Change |','|---:|---:|---:|---|---:|---:|---:|']
 for w,c,size in sorted(set((s['workers'],s['connections'],s['bytes']) for s in report['samples'])):
  for b in ['iouring','iouring-poll']:
   xs={v:[s for s in report['samples'] if (s['variant'],s['backend'],s['workers'],s['connections'],s['bytes'])==(v,b,w,c,size)] for v in ['before','after']}
   assert all(len(rows)==3 for rows in xs.values())
   med={v:statistics.median(s['requests_per_second'] for s in rows) for v,rows in xs.items()}
   lines.append(f"| {w} | {c} | {size} | {b} | {med['before']:,.0f} | {med['after']:,.0f} | {(med['after']/med['before']-1)*100:+.2f}% |")
 lines+=['']
idle=json.loads((p/'idle/idle.json').read_text());assert len(idle['samples'])==8
assert all(sum(s['cpu_seconds'].values())==0 for s in idle['samples'])
lines+=['## Idle CPU','','All eight original/final × owned/borrowed × one/four-worker samples recorded zero CPU ticks over their two-second idle windows. Final ring workers sleep in `io_cqring_wait`. `idle/idle.json` retains per-thread wait channels and binary hashes.','','## Separate notification mechanism proof','','The immediate predecessor candidate was measured with a separately instrumented binary before the final cancellation-queue idle guard and syscall test seam were added. All four owned/borrowed × one/four-worker cases recorded zero owner-controller eventfd writes, with successful remote writes (2, 2, 38,052, 26,201). Those instrumented throughput values are excluded from every table here. See `../core-scheduler-direct-notification-write-fastpath/counted/samples/aiko.json` and its archived instrumentation patch; this is not an instrumented build of the final executable.','','## Artifacts','','Every final timing sample has a uniquely named JSON file and stderr in its phase directory, alongside a complete aggregate `aiko.json`; no final sample metadata was reconstructed. `provenance.json` records all executable/source/runner hashes, toolchain, machine, affinity, and build settings. Prior experimental recovery is documented in its own directory and does not affect this fresh run.','','Frozen final executable SHA-256: `'+prov['binaries']['benchmark']['sha256']+'`. Original control SHA-256: `'+prov['binaries']['control']['sha256']+'`.','']
prov['artifact_hashes']={str(f.relative_to(p)):hashlib.sha256(f.read_bytes()).hexdigest() for f in [p/'source.patch',p/'source-commit.json',p/'run-final.py',p/'sample-runner.py',p/'summarize.py',p/'run-final.log',p/'concurrent/aiko.json',p/'serial64/aiko.json',p/'idle/idle.json']}
(p/'provenance.json').write_text(json.dumps(prov,indent=2)+'\n')
(p/'analysis.md').write_text('\n'.join(lines))
print('\n'.join(lines))
