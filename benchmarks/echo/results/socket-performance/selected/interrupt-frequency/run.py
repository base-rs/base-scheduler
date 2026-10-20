import argparse, fcntl, hashlib, importlib.util, json, os, random, subprocess, threading, time
from pathlib import Path

root=Path('/home/samuel/Developer/core-rs/core-scheduler-perf-idle')
spec=importlib.util.spec_from_file_location('runner', root/'benchmarks/echo/results/readiness-profiling/run.py')
r=importlib.util.module_from_spec(spec); spec.loader.exec_module(r)
out=Path('/home/samuel/.cache/core-scheduler-interrupt-frequency'); out.mkdir(exist_ok=False)
binaries={'before':Path('/tmp/core-perf-socket-control'), 'prototype':Path('/tmp/core-perf-socket-candidate'), 'read':Path('/tmp/core-perf-idle-fix-read')}
client=Path('/tmp/core-scheduler-profile-baseline/release/core-scheduler-echo-benchmark')
report={'binaries':{k:{'path':str(v),'sha256':hashlib.sha256(v.read_bytes()).hexdigest()} for k,v in binaries.items()},'client':{'path':str(client),'sha256':hashlib.sha256(client.read_bytes()).hexdigest()},'seed':20261020,'samples':[],'note':'Kernel counters span server lifetime after tracer readiness, including 1s warmup and 3s measurement. Client counts measurement-window requests only. No ptrace or source/build changes. Exclusive host lock.'}
original_popen=subprocess.Popen
args=argparse.Namespace(server=None, client=client, output=out, privileged_perf=False)
active=[]

def hook(command,*a,**kw):
    p=original_popen(command,*a,**kw)
    if not (isinstance(command,list) and 'server' in command and command[0]=='taskset'):
        return p
    if not instrumented:
        return p
    program=f'''BEGIN {{ printf("READY\\n"); }}
tracepoint:io_uring:io_uring_submit_req /pid == {p.pid}/ {{
 @rings[(uint64)args->ctx] = 1;
 @sqes[args->opcode, args->user_data == 0xffffffffffffffff] = count();
}}
tracepoint:io_uring:io_uring_complete /@rings[(uint64)args->ctx]/ {{
 @cqes[args->user_data == 0xffffffffffffffff, args->res] = count();
}}
tracepoint:syscalls:sys_enter_io_uring_enter /pid == {p.pid}/ {{
 @threads[tid] = 1;
 @enter[args->to_submit, args->min_complete, args->flags] = count();
}}
tracepoint:syscalls:sys_enter_write /pid == {p.pid}/ {{ @writefd[tid] = args->fd + 1; }}
tracepoint:syscalls:sys_exit_write /pid == {p.pid} && @writefd[tid]/ {{
 @writes[@writefd[tid]-1, args->ret] = count(); delete(@writefd[tid]);
}}
tracepoint:syscalls:sys_enter_read /pid == {p.pid}/ {{ @readfd[tid] = args->fd + 1; }}
tracepoint:syscalls:sys_exit_read /pid == {p.pid} && @readfd[tid]/ {{
 @reads[@readfd[tid]-1, args->ret] = count(); delete(@readfd[tid]);
}}
tracepoint:syscalls:sys_enter_sendto /pid == {p.pid}/ {{ @sends[args->fd] = count(); }}
tracepoint:syscalls:sys_enter_epoll_pwait /pid == {p.pid}/ {{ @epoll[tid] = count(); }}
tracepoint:syscalls:sys_enter_epoll_wait /pid == {p.pid}/ {{ @epoll[tid] = count(); }}
tracepoint:sched:sched_switch /@threads[args->prev_pid]/ {{ @switches[args->prev_pid,args->prev_state] = count(); }}
tracepoint:sched:sched_process_exit /pid == {p.pid} && tid == {p.pid}/ {{ exit(); }}
END {{ clear(@rings); clear(@threads); clear(@readfd); clear(@writefd); }}
'''
    program_path=out/(current_id+'.bt'); program_path.write_text(program)
    error=(out/(current_id+'.bpf.stderr')).open('w')
    tracer=original_popen(['sudo','-n','taskset','-c','8','bpftrace','-q',str(program_path)],stdout=subprocess.PIPE,stderr=error,text=True)
    lines=[]; ready=threading.Event()
    def consume():
        for line in tracer.stdout:
            lines.append(line)
            if 'READY' in line: ready.set()
    t=threading.Thread(target=consume,daemon=True); t.start()
    if not ready.wait(30):
        raise RuntimeError('BPF not ready: '+(out/(current_id+'.bpf.stderr')).read_text())
    fdmap={}
    for fd in (Path('/proc')/str(p.pid)/'fd').iterdir():
        try: fdmap[fd.name]=os.readlink(fd)
        except FileNotFoundError: pass
    active.append((tracer,t,lines,error,fdmap))
    return p

r.subprocess.Popen=hook
cases=[(rep,label,inst) for rep in range(1,3) for label in binaries for inst in [False,True]]
random.Random(report['seed']).shuffle(cases)
with Path('/tmp/core-scheduler-perf.lock').open('a') as lock:
    fcntl.flock(lock,fcntl.LOCK_EX)
    for rep,label,instrumented in cases:
        current_id=f'{label}-'+('bpf' if instrumented else 'control')+f'-r{rep}'
        args.server=binaries[label]
        folder=out/current_id; folder.mkdir(); args.output=folder
        result=r.sample(args,'counter' if instrumented else 'control','iouring',1,1,64,rep)
        result.update(variant=label,instrumented=instrumented)
        if instrumented:
            tracer,t,lines,error,fdmap=active.pop()
            tracer.wait(timeout=30);t.join(timeout=3);error.close()
            if tracer.returncode: raise RuntimeError('BPF failed')
            (out/(current_id+'.bpf.txt')).write_text(''.join(lines))
            result['fd_map']=fdmap
            result['counters']=''.join(lines)
        report['samples'].append(result)
        (out/'aiko.json').write_text(json.dumps(report,indent=2)+'\n')
        print(current_id,round(result['requests_per_second']),flush=True)
print('DONE',out,flush=True)
