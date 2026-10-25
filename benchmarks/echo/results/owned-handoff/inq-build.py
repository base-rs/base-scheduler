import fcntl,hashlib,json,subprocess
from pathlib import Path
cache=Path('/home/samuel/.cache/core-scheduler-owned-dispatch')
with open('/tmp/core-scheduler-perf.lock','a') as lock:
 fcntl.flock(lock,fcntl.LOCK_EX)
 source=cache/'inq.c'; binary=cache/'bin/inq'
 subprocess.run(['cc','-O3','-g','-Wall','-Wextra','-Werror','-pthread',str(source),'-luring','-o',str(binary)],check=True)
 (cache/'inq-build-manifest.json').write_text(json.dumps({'source_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'compiler':subprocess.check_output(['cc','--version'],text=True)},indent=2)+'\n')
 print('TCP_INQ kernel control built',flush=True)
