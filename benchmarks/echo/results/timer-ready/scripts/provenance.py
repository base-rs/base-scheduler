import fcntl,hashlib,json,subprocess
from pathlib import Path
home=Path.home();root=home/'.cache/core-scheduler-timer-ready'
with open('/tmp/core-scheduler-perf.lock','a') as lock:
 print('Waiting for final provenance lock',flush=True);fcntl.flock(lock,fcntl.LOCK_EX)
 report={'compiler':subprocess.check_output(['rustc','-Vv'],text=True),'kernel':subprocess.check_output(['uname','-srmo'],text=True),'topology':json.loads(subprocess.check_output(['lscpu','-J'],text=True)),'binaries':{}}
 for variant in ['immediate','ready','batch','ready-batch','ready-local']:
  build=json.loads((root/(variant+'-build.json')).read_text())
  for name,expected in build['binaries'].items():
   actual=hashlib.sha256((root/name).read_bytes()).hexdigest();assert actual==expected['sha256'];report['binaries'][name]=actual
 (root/'provenance.json').write_text(json.dumps(report,indent=2)+'\n');print('All frozen executable hashes verified',flush=True)
