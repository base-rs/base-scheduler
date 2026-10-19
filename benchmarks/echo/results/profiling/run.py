import argparse, itertools, json, os, queue, random, resource, signal, subprocess, threading, time
from pathlib import Path
ROOT=Path.home()/'Developer/core-rs/core-scheduler'
OUT=Path('/tmp/core-echo-profile');OUT.mkdir(exist_ok=True)
BASE=Path('/tmp/core-scheduler-profile-baseline/release/core-scheduler-echo-benchmark')
LTO=Path('/tmp/core-scheduler-profile-lto/release/core-scheduler-echo-benchmark')
CAPACITY=Path('/tmp/core-scheduler-profile-capacity/release/core-scheduler-echo-benchmark')
PROTOTYPE=Path('/tmp/core-scheduler-profile-prototype/release/core-scheduler-echo-benchmark')
def usage():
    r=resource.getrusage(resource.RUSAGE_CHILDREN)
    return dict(user=r.ru_utime,system=r.ru_stime,voluntary_switches=r.ru_nvcsw,involuntary_switches=r.ru_nivcsw)
def delta(a,b):return {k:b[k]-v for k,v in a.items()}
def sample(variant,backend,workers,connections,mode,repeat):
    ident=f'{mode}-{variant}-{backend}-w{workers}-c{connections}-r{repeat}'
    binary={'baseline':BASE,'lto':LTO,'prototype':PROTOTYPE,'capacity':CAPACITY}[variant]
    prefix=[]
    if mode=='perf':prefix=['perf','record','-q','-m','32','-e','cycles:u','-F','999','--call-graph','dwarf,8192','-o',str(OUT/(ident+'.data')),'--']
    if mode=='strace':prefix=['strace','-f','-c','-S','calls','-o',str(OUT/(ident+'.strace'))]
    warmup,seconds=(0,0.5) if mode=='strace' else (1,5 if mode=='perf' else 3)
    with open(OUT/(ident+'.stderr'),'w+') as errors:
        server=subprocess.Popen(prefix+['taskset','-c',','.join(map(str,range(workers))),str(binary),'server',backend,str(workers),str(connections)],cwd=ROOT,stdout=subprocess.PIPE,stderr=errors,text=True,start_new_session=True)
        try:
            ready=queue.Queue();threading.Thread(target=lambda:ready.put(server.stdout.readline()),daemon=True).start()
            address=ready.get(timeout=30).strip()
            if not address:raise RuntimeError('server failed')
            before=usage();started=time.monotonic()
            client=subprocess.run(['taskset','-c','4-7',str(BASE),'client',address,'4',str(connections),'64',str(warmup),str(seconds)],text=True,capture_output=True,check=True,timeout=seconds+warmup+60)
            after_client=usage()
            if server.wait(timeout=30):raise RuntimeError('server failed')
            result=dict(json.loads(client.stdout),variant=variant,backend=backend,workers=workers,repeat=repeat,mode=mode,server_binary=str(binary),client_binary=str(BASE),client_cpu=delta(before,after_client),server_cpu=delta(after_client,usage()),lifecycle_seconds=time.monotonic()-started)
            print(ident,round(result['requests_per_second']),'client_cpu',result['client_cpu'],'server_cpu',result['server_cpu'],flush=True)
            return result
        except BaseException:
            errors.seek(0);print(errors.read(),flush=True);raise
        finally:
            # perf/strace can exit while their server child still owns stdout.
            # Kill the whole session's process group, even if the wrapper exited.
            try:os.killpg(server.pid,signal.SIGKILL)
            except ProcessLookupError:pass
            server.wait();server.stdout.close()
p=argparse.ArgumentParser();p.add_argument('mode',choices=['compare','experiments','capacity','perf','strace']);p.add_argument('--variant',default='baseline');args=p.parse_args()
if args.mode in ['compare','experiments','capacity']:
    cases=list(itertools.product(range(1,4),[(1,1),(1,32),(4,32)],(['baseline','lto'] if args.mode=='compare' else ['baseline',('capacity' if args.mode=='capacity' else 'prototype')]),(['tokio','tokio-adapter','iouring'] if args.mode=='compare' else (['iouring'] if args.mode=='capacity' else ['tokio-adapter','iouring']))))
    random.Random(20261020).shuffle(cases)
else:cases=[(1,(w,32),args.variant,b) for w in ([1,4] if args.mode=='perf' else [1]) for b in ['tokio','tokio-adapter','iouring']]
report=dict(commit=subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),server_cpu_policy='first N of CPUs 0-3',client_cpus='4-7',client_binary=str(BASE),samples=[])
for repeat,(workers,connections),variant,backend in cases:
    report['samples'].append(sample(variant,backend,workers,connections,args.mode,repeat))
    (OUT/(args.mode+('-'+args.variant if args.variant!='baseline' else '')+'.json')).write_text(json.dumps(report,indent=2)+'\n')
