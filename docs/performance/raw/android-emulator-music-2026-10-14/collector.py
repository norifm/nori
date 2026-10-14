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
def session(require_playing=True):
    text=adb('shell','dumpsys','media_session')
    start=text.index('package='+pkg)
    result=text[start:text.find('\n  MediaSessionRecord',start+1) if '\n  MediaSessionRecord' in text[start+1:] else len(text)]
    if require_playing: assert 'state=PLAYING(3)' in result,result[:1000]
    assert 'bgtest' in result,result[-1000:]
    return result
hz=int(adb('shell','getconf','CLK_TCK').strip())
audio_pids=set(adb('shell','pidof','audioserver','media.swcodec','media.extractor').split())
assert len(audio_pids)==3, 'Android audio and codec services missing'
awk=r'''awk -v files="$files" 'BEGIN {
 n=split(files,paths," ");
 for(i=1;i<=n;i++) {
  path=paths[i];if(path=="")continue;
  split(path,p,"/");id=p[3] ":" p[5];
  while((getline line < path)>0) {
   if(path ~ /\/stat$/) {
    sub(/^.*\) /,"",line);split(line,a," ");
    if(path ~ /\/task\//)ticks[id]=a[12]+a[13];
    else print "P",p[3] ":" a[20],a[12]+a[13];
   } else if(line ~ /^voluntary_ctxt_switches:/) {split(line,a," ");switches[id]=a[2]}
  }
  close(path);
 }
 for(id in ticks)if(id in switches)print "T",id,ticks[id],switches[id];
}'
'''
def sample():
    before=time.monotonic()
    text=adb('shell',f'files=""; for p in $(pgrep -f "^{pkg}") $(pidof audioserver media.swcodec media.extractor); do for f in /proc/$p/stat /proc/$p/task/*/stat /proc/$p/task/*/status; do files="$files $f"; done; done; '+awk)
    after=time.monotonic()
    threads={}
    processes={}
    audio_threads={}
    audio_processes={}
    for line in text.splitlines():
        fields=line.split()
        kind,key,ticks=fields[:3]
        is_audio=key.split(':')[0] in audio_pids
        if kind=='P': (audio_processes if is_audio else processes)[key]=int(ticks)
        else: (audio_threads if is_audio else threads)[key]=(int(ticks),int(fields[3]))
    assert threads and processes and audio_processes, 'app or audio-service counters missing'
    return {'time':(before+after)/2,'threads':threads,'processes':processes,'audioThreads':audio_threads,'audioProcesses':audio_processes}
subprocess.run(['adb','-s',serial,'shell','input','keyevent','223'],check=True)
results=[]
sessions=[]
deadline=time.monotonic()+60
while True:
    before=session(False)
    assert 'state=ERROR(7)' not in before, before
    if 'state=PLAYING(3)' in before:
        position=int(re.search(r'position=(\d+)',before)[1])
        updated=int(re.search(r'updated=(\d+)',before)[1])
        uptime=float(adb('shell','cat','/proc/uptime').split()[0])*1000
        if position+max(0,uptime-updated)>=15000: break
    assert time.monotonic()<deadline, 'playback did not warm up'
    time.sleep(.2)
for run in range(2):
    before=session()
    first=sample()
    previous=first
    samples=[first]
    quiet=0
    audit=[]
    for index in range(1,301):
        deadline=first['time']+index
        delay=deadline-time.monotonic()
        if delay>0: time.sleep(delay)
        current=sample()
        delta=sum(current['threads'][tid][1]-previous['threads'][tid][1] for tid in current['threads'].keys() & previous['threads'].keys())
        assert delta>=0
        if current['threads'].keys()==previous['threads'].keys() and delta<12: quiet+=1
        samples.append(current)
        previous=current
        if index % 15 == 0:
            audit.append({'second':index,'session':session(False)})
            (out/'live-session.json').write_text(json.dumps(audit[-1],indent=2)+'\n')
        if index % 60 == 0: print(f'run {run+1}: {index}/300 seconds',flush=True)
    after=session()
    sessions.append({"run":run+1,"before":before,"after":after,"audit":audit})
    (out/"sessions.json").write_text(json.dumps(sessions,indent=2)+"\n")
    last=samples[-1]
    common=first['threads'].keys() & last['threads'].keys()
    ticks=0
    audio_ticks=0
    switches=0
    for a,b in zip(samples,samples[1:]):
        matched=a['threads'].keys() & b['threads'].keys()
        process_matched=a['processes'].keys() & b['processes'].keys()
        ticks+=sum(b['processes'][pid]-a['processes'][pid] for pid in process_matched)
        audio_matched=a['audioProcesses'].keys() & b['audioProcesses'].keys()
        audio_ticks+=sum(b['audioProcesses'][pid]-a['audioProcesses'][pid] for pid in audio_matched)
        switches+=sum(b['threads'][tid][1]-a['threads'][tid][1] for tid in matched)
    elapsed=last['time']-first['time']
    mem=adb('shell','dumpsys','meminfo',pkg)
    pss=int(re.search(r'TOTAL PSS:\s*(\d+)',mem)[1])/1024
    result={'run':run+1,'seconds':elapsed,'cpuPercent':ticks/hz/elapsed*100,'audioCpuPercent':audio_ticks/hz/elapsed*100,'combinedCpuPercent':(ticks+audio_ticks)/hz/elapsed*100,'wakeupsPerSecond':switches/elapsed,'quietIntervals':quiet,'intervals':300,'pssMiB':pss,'threadsStart':len(first['threads']),'threadsEnd':len(last['threads']),'threadsMatched':len(common)}
    (out/f'playback-{run+1}-samples.json').write_text(json.dumps(samples,indent=2)+'\n')
    results.append(result)
    (out/'playback-results.json').write_text(json.dumps(results,indent=2)+'\n')
    print(json.dumps(result),flush=True)
