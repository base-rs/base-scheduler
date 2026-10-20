from pathlib import Path
import hashlib,json,os,socket,subprocess,signal,time
base=Path('/home/samuel/.cache/core-scheduler-ring-notification-results/fixed-idle')
bins={'before':base.parent/'control','after':base.parent/'benchmark-fixed'}
def stat(pid):
    fields=Path(f'/proc/{pid}/stat').read_text().rsplit(')',1)[1].split()
    return {'user_ticks':int(fields[11]),'system_ticks':int(fields[12])}
def channels(pid):
    return [{'tid':p.name,'name':(p/'comm').read_text().strip(),'wait':(p/'wchan').read_text().strip()} for p in Path(f'/proc/{pid}/task').iterdir()]
report={'binary_hashes':{k:hashlib.sha256(p.read_bytes()).hexdigest() for k,p in bins.items()},'ticks_per_second':os.sysconf('SC_CLK_TCK'),'samples':[]}
for variant,binary in bins.items():
 for workers in [1,4]:
  for backend in ['iouring','iouring-poll']:
   connections=workers
   process=subprocess.Popen(['taskset','-c',','.join(map(str,range(workers))),str(binary),'server',backend,str(workers),str(connections)],stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,start_new_session=True)
   clients=[]
   try:
    address=process.stdout.readline().strip();host,port=address.rsplit(':',1)
    clients=[socket.create_connection((host,int(port)),timeout=5) for _ in range(connections)]
    time.sleep(.2)
    before=stat(process.pid);start=time.monotonic();waits=channels(process.pid)
    time.sleep(2)
    after=stat(process.pid);elapsed=time.monotonic()-start
    row={'variant':variant,'backend':backend,'workers':workers,'connections':connections,'elapsed':elapsed,'cpu_seconds':{k:(after[k]-before[k])/report['ticks_per_second'] for k in before},'wait_channels':waits}
    report['samples'].append(row);print(variant,workers,backend,row['cpu_seconds'],flush=True)
    (base/'idle.json').write_text(json.dumps(report,indent=2)+'\n')
    for s in clients:s.close()
    clients=[]
    process.wait(timeout=10)
    if process.returncode:raise RuntimeError(process.stderr.read())
   finally:
    for s in clients:s.close()
    if process.poll() is None:
     os.killpg(process.pid,signal.SIGKILL);process.wait()
    process.stdout.close();process.stderr.close()
