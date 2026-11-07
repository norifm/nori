import argparse,subprocess,sys
from pathlib import Path
p=argparse.ArgumentParser()
p.add_argument('app',choices=['nori','symfonium','musly','navic'])
p.add_argument('--serial',default='emulator-5556')
p.add_argument('--server',default='http://10.0.2.2:4541',help='the Navidrome as the device reaches it')
p.add_argument('--output',default='build/website/emulator-music-replay')
a=p.parse_args()
pkg={'nori':'dev.nori.music.perf','symfonium':'app.symfonik.music.player','musly':'com.devid.musly','navic':'paige.navic'}[a.app]
component={'nori':pkg+'/dev.nori.music.app.MainActivity','symfonium':pkg+'/app.symfonik.ui.MainActivity','musly':pkg+'/.MainActivity','navic':pkg+'/.MainActivityDefault'}[a.app]
out=a.output+'/'+a.app
def adb(*v,**k): return subprocess.run(['adb','-s',a.serial,'shell',*v],**k)
# A phone's own settings (keyboards, screen timeout, volume, languages) are put back however the run ends.
sys.path.insert(0,str(Path('tools/bgtest').resolve()))
import ui,phone
ui.SERIAL=a.serial
device=phone.Phone([pkg]);device.save();device.prepare(False)
subprocess.run([sys.executable,'tools/bgtest/compare/start.py',a.app,a.serial,a.server,a.output],check=True)
adb('svc','power','stayon','false',check=True)
# The lowest step, nearly silent: some phones mute the stream at 0.
adb('cmd','media_session','volume','--stream','3','--set','1',stdout=subprocess.DEVNULL)
subprocess.run([sys.executable,'tools/bgtest/compare/collector.py','--serial',a.serial,'--package',pkg,'--output',out],check=True)
adb('am','force-stop',pkg,check=True)
subprocess.run([sys.executable,'tools/bgtest/compare/cold-start.py','--serial',a.serial,'--package',pkg,'--component',component,'--output',out+'/cold-start.json'],check=True)
adb('am','force-stop',pkg,check=True)
