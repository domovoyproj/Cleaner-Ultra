"""Verify the packaged Rust server's HEIC and RAW scan with generated fixtures."""
import json
import subprocess
import urllib.request
from pathlib import Path

root = Path(__file__).resolve().parent.parent
fixtures = root / 'build' / 'format-fixtures'
magick = root / '.cleaner_ultra' / 'tools' / 'magick' / 'magick.exe'
# HEIC writer in the portable codec build is intentionally read-only; generate with ffmpeg/libheif sample tests when available.
def check(base):
    def request(route, body=None, token=''):
        headers={'Content-Type':'application/json','X-CleanerUltra-Token':token}
        req=urllib.request.Request(base+route,headers=headers,data=None if body is None else json.dumps(body).encode())
        with urllib.request.urlopen(req,timeout=120) as response:
            return json.load(response)
    token=request('/api/session')['token']
    request('/api/scan',{'path':str(fixtures)},token)
    import time
    for _ in range(120):
        progress=request('/api/progress')
        if not progress['running']:
            break
        time.sleep(.25)
    assert progress['errors']==0,progress
    photos=request('/api/photos')['photos']
    raw=next(p for p in photos if p['name']=='TEST_bayer.dng')
    assert raw['width']>0 and raw['height']>0
    with urllib.request.urlopen(base+f"/media/{raw['id']}?full=1") as response:
        assert response.read(2)==b'\xff\xd8'
    heic=next(p for p in photos if p['name']=='TEST_libheif.heic')
    assert heic['width']>0 and heic['height']>0
    with urllib.request.urlopen(base+f"/media/{heic['id']}?full=1") as response:
        assert response.read(2)==b'\xff\xd8'
    print('RAW DNG and HEIC scan and full preview: OK')

if __name__=='__main__':
    import sys
    check(sys.argv[1])
