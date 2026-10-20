from pathlib import Path
import json,statistics
root=Path('/home/samuel/.cache/core-scheduler-native-serial-next');report=json.loads((root/'serial/aiko.json').read_text());summary={}
for key in ['before','selected','no-coop']:
 samples=[s for s in report['samples'] if s['variant']==key];rates=[s['requests_per_second'] for s in samples]
 summary[key]={'rates':rates,'median':statistics.median(rates),'mean':statistics.mean(rates),'minimum':min(rates),'maximum':max(rates),'standard_deviation':statistics.stdev(rates),'median_rtt_mean_us':statistics.median(s['rtt_mean_us'] for s in samples),'median_rtt_p50_us':statistics.median(s['rtt_p50_us'] for s in samples),'median_server_user_cpu':statistics.median(s['server_cpu']['user'] for s in samples),'median_server_system_cpu':statistics.median(s['server_cpu']['system'] for s in samples),'median_lifecycle_voluntary_switches':statistics.median(s['server_cpu']['voluntary_switches'] for s in samples)}
comparisons={}
for a,b in [('selected','before'),('no-coop','before'),('no-coop','selected')]:comparisons[a+'/'+b]={metric:100*(summary[a][metric]/summary[b][metric]-1) for metric in ['median','mean']}
output={'variants':summary,'changes_percent':comparisons};(root/'summary.json').write_text(json.dumps(output,indent=2)+'\n');print(json.dumps(output,indent=2))
text='''# Native borrowed serial cooperative-ring diagnostic

Five randomized repetitions per frozen binary, exclusive shared performance lock, one-second warmup and three-second measurement, default optimized release builds, fixed client,1worker/1connection/64bytes. The no-coop variant differs from selected source only by removing `setup_coop_taskrun` and `setup_taskrun_flag`; started tracking and replacement-ring construction remain.

| Binary | Median requests/s | Mean requests/s | Range requests/s |
|---|---:|---:|---:|
'''
for key,s in summary.items():text+=f"| {key} | {s['median']:,.0f} | {s['mean']:,.0f} | {s['minimum']:,.0f}–{s['maximum']:,.0f} |\n"
text+='''
Selected is2.61%below before by median but1.46%by mean. Removing the cooperative flags recovers1.56%by median and only0.32%by mean, leaving no-coop1.09%below before by median. All three sample ranges overlap substantially. These15samples support a small serial tradeoff being possible, but do not establish the previously observed4.2%median gap as the causal cost of cooperative flags or justify an adaptive policy. Between-run variability is similar in scale to the observed differences.

`serial/aiko.json` and individual sampleJSON/stderr retain every sample; `summary.json` includes means, medians, ranges, RTT and lifecycleCPU/context-switch values. CPU/context-switch fields include warmup and teardown, while rates/RTT cover the measured window. `provenance.json` records binary/source/runner hashes, the exact isolated two-line flags delta, build settings/toolchain/topology and affinity. No production changes were selected or committed.
'''
(root/'analysis.md').write_text(text)
