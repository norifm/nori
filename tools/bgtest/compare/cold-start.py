import argparse,subprocess,json,re,statistics
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('--package',required=True);p.add_argument('--component',required=True);p.add_argument('--output',required=True,type=Path);a=p.parse_args()
def adb(*v): return subprocess.check_output(['adb','-s','emulator-5556','shell',*v],text=True)
adb('input','keyevent','224');adb('wm','dismiss-keyguard')
runs=[];logs=[]
for i in range(5):
 adb('am','force-stop',a.package)
 log=adb('am','start','-W','-n',a.component)
 assert 'Status: ok' in log and 'LaunchState: COLD' in log,log
 runs.append(int(re.search(r'TotalTime: (\d+)',log)[1]));logs.append(log)
result={'runsMs':runs,'medianMs':statistics.median(runs),'logs':logs}
a.output.parent.mkdir(parents=True,exist_ok=True);a.output.write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result))
