import argparse
import io
import json
import secrets
import subprocess
import sys
import threading
import webbrowser
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlparse

from PIL import Image, ImageOps

from .library import Library

WEB = Path(__file__).parent / 'web'


def make_server(library, port=8765, folder_picker=None):
    token = secrets.token_urlsafe(32)

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def send(self, data, content_type='application/json', status=200):
            if not isinstance(data, bytes):
                data = json.dumps(data, ensure_ascii=False).encode('utf-8')
            self.send_response(status)
            self.send_header('Content-Type', content_type)
            self.send_header('Content-Length', str(len(data)))
            self.send_header('Cache-Control', 'no-store')
            self.send_header('X-Content-Type-Options', 'nosniff')
            self.send_header('Content-Security-Policy', "default-src 'self'; img-src 'self' data:; style-src 'self'; script-src 'self'; frame-ancestors 'none'")
            self.end_headers()
            try:
                self.wfile.write(data)
            except (BrokenPipeError, ConnectionResetError):
                pass

        def allowed(self):
            return self.headers.get('Host') == f'127.0.0.1:{self.server.server_port}'

        def do_GET(self):
            if not self.allowed():
                self.send({'error': 'Недопустимый адрес'}, status=403)
                return
            url = urlparse(self.path)
            query = parse_qs(url.query)
            try:
                if url.path == '/api/session':
                    self.send({'token': token})
                elif url.path == '/api/photos':
                    self.send(library.listing(query.get('view', ['all'])[0], query.get('search', [''])[0],
                                              int(query.get('offset', [0])[0]), query.get('sort', ['date'])[0]))
                elif url.path == '/api/progress':
                    self.send(dict(library.progress))
                elif url.path.startswith('/media/'):
                    photo = library.photo(int(url.path.split('/')[-1]))
                    if query.get('full'):
                        with Image.open(photo['trash'] or photo['path']) as original:
                            image = ImageOps.exif_transpose(original).convert('RGB')
                            image.thumbnail((2400, 1800))
                            buffer = io.BytesIO()
                            image.save(buffer, 'JPEG', quality=92)
                            self.send(buffer.getvalue(), 'image/jpeg')
                    else:
                        self.send((library.thumbs / f"{photo['hash']}.jpg").read_bytes(), 'image/jpeg')
                elif url.path in {'/', '/app.js', '/style.css'}:
                    name = 'index.html' if url.path == '/' else url.path[1:]
                    mime = {'index.html': 'text/html; charset=utf-8', 'app.js': 'text/javascript; charset=utf-8',
                            'style.css': 'text/css; charset=utf-8'}[name]
                    self.send((WEB / name).read_bytes(), mime)
                else:
                    self.send({'error': 'Не найдено'}, status=404)
            except (OSError, ValueError) as error:
                self.send({'error': str(error)}, status=400)

        def do_POST(self):
            if not self.allowed() or self.headers.get('X-CleanerUltra-Token') != token:
                self.send({'error': 'Доступ запрещён'}, status=403)
                return
            try:
                length = int(self.headers.get('Content-Length', 0))
                if length > 65536:
                    raise ValueError('Слишком большой запрос')
                body = json.loads(self.rfile.read(length) or b'{}')
                route = urlparse(self.path).path
                if route == '/api/scan':
                    library.start_scan(body.get('path', ''))
                    result = {'ok': True}
                elif route == '/api/stop':
                    library.cancel.set()
                    result = {'ok': True}
                elif route == '/api/pick':
                    if folder_picker is not None:
                        self.send({'path': folder_picker()})
                        return
                    script = "import tkinter as t; from tkinter import filedialog; r=t.Tk(); r.withdraw(); r.attributes('-topmost',True); print(filedialog.askdirectory(title='Выберите папку с фотографиями')); r.destroy()"
                    picked = subprocess.run([sys.executable, '-c', script], capture_output=True, text=True,
                                            encoding='utf-8', env={**__import__('os').environ, 'PYTHONIOENCODING': 'utf-8'},
                                            timeout=180)
                    if picked.returncode:
                        raise ValueError('Диалог недоступен. Введите путь вручную')
                    result = {'path': picked.stdout.strip()}
                elif route in {'/api/favorite', '/api/trash', '/api/restore'}:
                    ids = body.get('ids', [])
                    if not isinstance(ids, list) or len(ids) > 1000 or any(type(i) is not int for i in ids):
                        raise ValueError('Некорректный список фотографий')
                    if route == '/api/favorite':
                        library.favorite(ids, body.get('value', True))
                        result = {'ok': True}
                    else:
                        result = library.move(ids, restore=route == '/api/restore')
                else:
                    self.send({'error': 'Не найдено'}, status=404)
                    return
                self.send(result)
            except (OSError, ValueError, subprocess.TimeoutExpired) as error:
                self.send({'error': str(error)}, status=400)

    return ThreadingHTTPServer(('127.0.0.1', port), Handler)


def main():
    parser = argparse.ArgumentParser(description='CleanerUltra — локальная фотобиблиотека')
    parser.add_argument('--port', type=int, default=8765)
    parser.add_argument('--data', default=str(Path(__file__).resolve().parent.parent / '.cleaner_ultra'))
    parser.add_argument('--no-browser', action='store_true')
    args = parser.parse_args()
    library = Library(args.data)
    server = make_server(library, args.port)
    url = f'http://127.0.0.1:{server.server_port}'
    print(f'CleanerUltra: {url}', flush=True)
    if not args.no_browser:
        threading.Timer(0.6, webbrowser.open, args=(url,)).start()
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        library.close()
        server.server_close()


if __name__ == '__main__':
    main()
