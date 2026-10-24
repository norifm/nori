import sys
from pathlib import Path
sys.path.insert(0,str(Path('tools/bgtest').resolve()))
import ui
from apps.nori import Nori
from apps.symfonium import Symfonium
from apps.musly import Musly
from apps.navic import Navic
ui.SERIAL='emulator-5556'
ui.screenshot=lambda *_: None
app={'nori':Nori,'symfonium':Symfonium,'musly':Musly,'navic':Navic}[sys.argv[1]](('http://10.0.2.2:4541','admin','admin'),'website-bench-mixed',print,str(Path('build/website/emulator-music-replay').resolve()))
ui.sh('input keyevent 224');ui.sh('wm dismiss-keyguard');ui.sh('svc power stayon true')
component={'nori':app.pkg+'/dev.nori.music.app.MainActivity','symfonium':app.pkg+'/app.symfonik.ui.MainActivity','musly':app.pkg+'/.MainActivity','navic':app.pkg+'/.MainActivityDefault'}[sys.argv[1]]
app.launch=lambda:ui.sh('am start -n '+component)
for pkg in ['dev.nori.music.perf','app.symfonik.music.player','com.devid.musly','paige.navic']:ui.sh('am force-stop '+pkg)
app.clean();app.login();app.start_playlist()
