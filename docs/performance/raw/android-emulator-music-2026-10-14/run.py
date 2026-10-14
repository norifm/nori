import subprocess,sys
name=sys.argv[1]
pkg={'nori':'dev.nori.music.perf','symfonium':'app.symfonik.music.player','musly':'com.devid.musly','navic':'paige.navic'}[name]
subprocess.run([sys.executable,'docs/performance/raw/android-emulator-music-2026-10-14/start.py',name],check=True)
subprocess.run(['adb','-s','emulator-5556','shell','svc','power','stayon','false'],check=True)
subprocess.run(['adb','-s','emulator-5556','shell','cmd','media_session','volume','--stream','3','--set','0'],stdout=subprocess.DEVNULL)
subprocess.run([sys.executable,'docs/performance/raw/android-emulator-music-2026-10-14/collector.py','--package',pkg,'--output','build/website/emulator-music-replay/'+name],check=True)
subprocess.run(['adb','-s','emulator-5556','shell','am','force-stop',pkg],check=True)
component={'nori':pkg+'/dev.nori.music.app.MainActivity','symfonium':pkg+'/app.symfonik.ui.MainActivity','musly':pkg+'/.MainActivity','navic':pkg+'/.MainActivityDefault'}[name]
subprocess.run([sys.executable,'docs/performance/raw/android-emulator-music-2026-10-14/cold-start.py','--package',pkg,'--component',component,'--output','build/website/emulator-music-replay/'+name+'/cold-start.json'],check=True)
subprocess.run(['adb','-s','emulator-5556','shell','am','force-stop',pkg],check=True)
