import argparse,subprocess,json,time,re
from pathlib import Path
parser=argparse.ArgumentParser()
parser.add_argument('--serial',default='emulator-5556')
parser.add_argument('--package',required=True)
parser.add_argument('--output',required=True,type=Path)
args=parser.parse_args()
serial=args.serial
pkg=args.package
out=args.output
out.mkdir(parents=True,exist_ok=True)
def adb(*args):
    return subprocess.check_output(['adb','-s',serial,*args],text=True)
def session():
    text=adb('shell','dumpsys','media_session')
    start=text.index('package='+pkg)
    result=text[start:text.find('\n  MediaSessionRecord',start+1) if '\n  MediaSessionRecord' in text[start+1:] else len(text)]
    assert 'state=PLAYING(3)' in result,result[:1000]
    assert 'Bench Pulse MP3' in result,result[-1000:]
    return result
hz=int(adb('shell','getconf','CLK_TCK').strip())
awk=r'''awk 'FILENAME ~ /\/stat$/ {s=$0;sub(/^.*\) /,"",s);split(s,a," ");split(FILENAME,p,"/");id=p[3] ":" p[5];ticks[id]=a[12]+a[13]} /^voluntary_ctxt_switches:/ {split(FILENAME,p,"/");switches[p[3] ":" p[5]]=$2} END {for(id in ticks) print id,ticks[id],switches[id]+0}' $files'''
def sample():
    before=time.monotonic()
    text=adb('shell',f'files=""; for p in $(pgrep -f "^{pkg}"); do files="$files /proc/$p/task/*/stat /proc/$p/task/*/status"; done; '+awk)
    after=time.monotonic()
    values={}
    for line in text.splitlines():
        tid,ticks,switches=line.split()
        values[tid]=(int(ticks),int(switches))
    assert values,'no app threads were readable'
    return {'time':(before+after)/2,'threads':values}
subprocess.run(['adb','-s',serial,'shell','input','keyevent','223'],check=True)
results=[]
sessions=[]
while True:
    before=session()
    position=int(re.search(r'position=(\d+)',before)[1])
    updated=int(re.search(r'updated=(\d+)',before)[1])
    uptime=float(adb('shell','cat','/proc/uptime').split()[0])*1000
    if position+max(0,uptime-updated)>=15000: break
    time.sleep(.2)
for run in range(2):
    before=session()
    first=sample()
    previous=first
    samples=[first]
    quiet=0
    for index in range(1,91):
        deadline=first['time']+index
        delay=deadline-time.monotonic()
        if delay>0: time.sleep(delay)
        current=sample()
        delta=sum(current['threads'][tid][1]-previous['threads'][tid][1] for tid in current['threads'].keys() & previous['threads'].keys())
        assert delta>=0
        if current['threads'].keys()==previous['threads'].keys() and delta<12: quiet+=1
        samples.append(current)
        previous=current
    after=session()
    sessions.append({"run":run+1,"before":before,"after":after})
    (out/"sessions.json").write_text(json.dumps(sessions,indent=2)+"\n")
    last=samples[-1]
    common=first['threads'].keys() & last['threads'].keys()
    ticks=0
    switches=0
    for a,b in zip(samples,samples[1:]):
        matched=a['threads'].keys() & b['threads'].keys()
        ticks+=sum(b['threads'][tid][0]-a['threads'][tid][0] for tid in matched)
        switches+=sum(b['threads'][tid][1]-a['threads'][tid][1] for tid in matched)
    elapsed=last['time']-first['time']
    mem=adb('shell','dumpsys','meminfo',pkg)
    pss=int(re.search(r'TOTAL PSS:\s*(\d+)',mem)[1])/1024
    result={'run':run+1,'seconds':elapsed,'cpuPercent':ticks/hz/elapsed*100,'wakeupsPerSecond':switches/elapsed,'quietIntervals':quiet,'intervals':90,'pssMiB':pss,'threadsStart':len(first['threads']),'threadsEnd':len(last['threads']),'threadsMatched':len(common)}
    (out/f'mp3-{run+1}-samples.json').write_text(json.dumps(samples,indent=2)+'\n')
    results.append(result)
    (out/'mp3-results.json').write_text(json.dumps(results,indent=2)+'\n')
    print(json.dumps(result),flush=True)
