import importlib.util,pathlib,types,json,subprocess,hashlib,time,os,re,gzip,shutil,platform
base=pathlib.Path('/home/samuel/Developer/core-rs/core-scheduler-perf-tokio')
spec=importlib.util.spec_from_file_location('runner',base/'benchmarks/echo/results/readiness-profiling/run.py');m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
out=pathlib.Path('/tmp/core-perf-tokio-final-syscalls');out.mkdir(exist_ok=False)
server=pathlib.Path('/tmp/core-perf-socket-selected');client=pathlib.Path('/tmp/core-scheduler-profile-baseline/release/core-scheduler-echo-benchmark')
expected={'server':'f5f0336a61e7ccfbf6db35b1ac10cf04845137465ef4250c0420691b1173908e','client':'db6c914ab75011477ec6232e681046b1e75532cbaa1c81663bac984591675cf3'}
report={'sources':{'server':'938e7e641f8d186991a0f46f211f4ce3d19c929e','client':'5736301fded46c4e5f11d5387950ae2ee0807b94'},'binaries':{k:{'path':str(p),'sha256':hashlib.sha256(p.read_bytes()).hexdigest()} for k,p in [('server',server),('client',client)]},'started_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),'hostname':platform.node(),'platform':platform.platform(),'strace_version':subprocess.check_output(['strace','--version'],text=True).splitlines()[0],'server_cpus':'0','client_cpus':'4-7','measurement_seconds':0.5,'warmup_seconds':0,'trace_syscalls':['read','write','readv','writev','recvfrom','sendto','recvmsg','sendmsg','accept','accept4','close'],'samples':[]}
for k,sha in expected.items():assert report['binaries'][k]['sha256']==sha,(k,report['binaries'][k])
original_popen=m.subprocess.Popen
def traced_popen(command,*args,**kwargs):
 command=list(command)
 if command and command[0]=='strace':
  command[command.index('-c')]='-C'
  command[1:1]=['-s','0','-e','trace='+','.join(report['trace_syscalls'])]
 return original_popen(command,*args,**kwargs)
m.subprocess.Popen=traced_popen
args=types.SimpleNamespace(client=client,server=server,privileged_perf=False,perf_event='cycles:u')
for backend in ['tokio','tokio-adapter','tokio-adapter-poll']:
 args.output=out/backend;args.output.mkdir()
 s=m.sample(args,'strace',backend,1,32,64,1);report['samples'].append(s)
 (out/'aiko.json').write_text(json.dumps(report,indent=2)+'\n');print(backend,s['requests'],flush=True)
# Export and compress only after all timed traces finish, while retaining the lock.
io_names={'read','write','readv','writev','recvfrom','sendto','recvmsg','sendmsg'}
for s in report['samples']:
 backend=s['backend'];folder=out/backend;trace=next(folder.glob('*.strace'));pending={};active=set();accepted=set();stats={};whole={};summary=[];in_summary=False
 def accumulate(table,name,ret,errno):
  x=table.setdefault(name,{'calls':0,'eagain':0,'other_errors':0,'zero_returns':0,'positive_returns':0,'bytes':0})
  x['calls']+=1
  if errno in ('EAGAIN','EWOULDBLOCK'):x['eagain']+=1
  elif ret<0:x['other_errors']+=1
  elif ret==0:x['zero_returns']+=1
  else:x['positive_returns']+=1;x['bytes']+=ret
 for line in trace.read_text().splitlines():
  if line.lstrip().startswith('% time'):in_summary=True
  if in_summary:summary.append(line);continue
  match=re.match(r'^(\d+)\s+(.*)$',line)
  if not match:continue
  tid,body=match.groups()
  resumed=re.match(r'<\.\.\. (\w+) resumed>(.*)',body)
  direct=re.match(r'(\w+)\((.*)',body)
  if resumed:
   name,tail=resumed.groups();previous=pending.pop(tid,None)
   if previous is None:continue
   assert name==previous[0],(line,previous)
   fd=previous[1]
  elif direct:
   name,tail=direct.groups();fdm=re.match(r'(\d+)',tail);fd=int(fdm.group(1)) if fdm else None
   if '<unfinished ...>' in tail:pending[tid]=(name,fd);continue
  else:continue
  returned=re.search(r'=\s+(-?\d+)\b(?:\s+([A-Z][A-Z0-9_]+))?',tail)
  if not returned:continue
  ret=int(returned.group(1));errno=returned.group(2)
  if name in ('accept','accept4') and ret>=0:active.add(ret);accepted.add(ret)
  elif name=='close' and ret==0:active.discard(fd)
  elif name in io_names:
   accumulate(whole,name,ret,errno)
   if fd in active:accumulate(stats,name,ret,errno)
 socket_groups={}
 for group,names in [('receive',['read','readv','recvfrom','recvmsg']),('send',['write','writev','sendto','sendmsg'])]:
  x={k:sum(stats.get(name,{}).get(k,0) for name in names) for k in ['calls','eagain','other_errors','zero_returns','positive_returns','bytes']}
  x['calls_per_completed_request']=x['calls']/s['requests'];x['eagain_per_completed_request']=x['eagain']/s['requests'];socket_groups[group]=x
 parsed={'accepted_socket_count':len(accepted),'accepted_socket_descriptors':sorted(accepted),'accepted_socket_io':stats,'accepted_socket_groups':socket_groups,'whole_process_io':whole,'pending_at_end':pending,'normalization_completed_client_requests':s['requests'],'notes':['Counts are whole-process-lifecycle totals; accepted-socket filtering excludes loader, stdout, notifier and other unrelated I/O.','Terminal EOF receives and any partial I/O remain included and are reported separately.','Strace changes scheduling and readiness timing. These counts describe the traced workload and cannot establish uninstrumented throughput or the fraction of the remaining throughput gap.']}
 s['syscalls']=parsed
 (folder/'syscalls.json').write_text(json.dumps(parsed,indent=2)+'\n')
 (folder/'summary.strace').write_text('\n'.join(summary)+'\n')
 with trace.open('rb') as src,gzip.open(str(trace)+'.gz','wb') as dst:shutil.copyfileobj(src,dst)
 trace.unlink()
 print(backend,json.dumps(socket_groups),flush=True)
report['finished_utc']=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime());(out/'aiko.json').write_text(json.dumps(report,indent=2)+'\n')
print('DONE',str(out),flush=True)
