from pathlib import Path
from PyInstaller.utils.hooks import collect_data_files

root = Path(SPECPATH)
a = Analysis([str(root / 'desktop_entry.py')], pathex=[str(root)],
    datas=[(str(root / 'cleaner_ultra' / 'web'), 'cleaner_ultra/web')] + collect_data_files('webview'),
    hiddenimports=['webview.platforms.edgechromium'], excludes=['pytest', 'graphify'])
pyz = PYZ(a.pure)
exe = EXE(pyz, a.scripts, [], exclude_binaries=True, name='CleanerUltra',
    console=False, icon=str(root / 'packaging' / 'cleaner_ultra.ico'))
coll = COLLECT(exe, a.binaries, a.datas, name='CleanerUltra')
