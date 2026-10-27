from __future__ import annotations

import hashlib
import os
import sqlite3
import threading
import time
import uuid
from contextlib import contextmanager
from datetime import datetime
from pathlib import Path

from PIL import Image, ImageOps, ImageFilter, ImageStat

EXTENSIONS = {'.jpg', '.jpeg', '.png', '.webp', '.tif', '.tiff', '.bmp'}


def fingerprint(path):
    digest = hashlib.sha256()
    with open(path, 'rb') as source:
        for block in iter(lambda: source.read(1024 * 1024), b''):
            digest.update(block)
    return digest.hexdigest()


def describe(path, thumbnail):
    with Image.open(path) as original:
        exif = original.getexif()
        captured = None
        try:
            captured = datetime.strptime(str(exif.get(36867) or exif.get(306)), '%Y:%m:%d %H:%M:%S').timestamp()
        except (ValueError, TypeError, OverflowError):
            pass
        image = ImageOps.exif_transpose(original).convert('RGB')
        width, height = image.size
        small = image.resize((9, 8)).convert('L')
        pixels = list(small.tobytes())
        dhash = sum((pixels[y * 9 + x] > pixels[y * 9 + x + 1]) << (y * 8 + x)
                    for y in range(8) for x in range(8))
        sample = ImageOps.fit(image, (128, 128)).convert('L')
        sharpness = round(ImageStat.Stat(sample.filter(ImageFilter.FIND_EDGES)).var[0], 2)
        image.thumbnail((640, 480))
        image.save(thumbnail, 'JPEG', quality=85)
    return width, height, captured, f'{dhash:016x}', sharpness


class BKTree:
    """Metric index avoids comparing every pair of photos."""

    def __init__(self):
        self.root = None

    def add(self, value, photo_id):
        node = self.root
        if node is None:
            self.root = (value, [photo_id], {})
            return
        while True:
            distance = (value ^ node[0]).bit_count()
            if distance == 0:
                node[1].append(photo_id)
                return
            if distance not in node[2]:
                node[2][distance] = (value, [photo_id], {})
                return
            node = node[2][distance]

    def find(self, value, radius=6):
        stack = [self.root] if self.root else []
        while stack:
            node = stack.pop()
            distance = (value ^ node[0]).bit_count()
            if distance <= radius:
                yield from node[1]
            stack.extend(child for edge, child in node[2].items()
                         if distance - radius <= edge <= distance + radius)


class Library:
    def __init__(self, data_dir):
        self.data_dir = Path(data_dir).resolve()
        self.data_dir.mkdir(parents=True, exist_ok=True)
        self.thumbs = self.data_dir / 'thumbnails'
        self.thumbs.mkdir(exist_ok=True)
        self.lock = threading.RLock()
        self.cancel = threading.Event()
        self.worker = None
        self.progress = {'running': False, 'done': 0, 'errors': 0, 'message': 'Готово к работе'}
        with self.connect() as db:
            db.executescript('''
                PRAGMA journal_mode=WAL;
                CREATE TABLE IF NOT EXISTS photos (
                    id INTEGER PRIMARY KEY, path TEXT UNIQUE NOT NULL, root TEXT NOT NULL,
                    name TEXT, size INTEGER, mtime INTEGER, width INTEGER, height INTEGER,
                    captured REAL, hash TEXT, dhash TEXT, sharpness REAL,
                    favorite INTEGER DEFAULT 0, trash TEXT, missing INTEGER DEFAULT 0,
                    similar INTEGER, burst INTEGER, seen TEXT);
                CREATE INDEX IF NOT EXISTS photos_hash ON photos(hash);
                CREATE INDEX IF NOT EXISTS photos_similar ON photos(similar);
                CREATE INDEX IF NOT EXISTS photos_burst ON photos(burst);
                CREATE TABLE IF NOT EXISTS operations (
                    id INTEGER PRIMARY KEY, photo INTEGER, source TEXT, destination TEXT,
                    kind TEXT, state TEXT DEFAULT 'pending');
            ''')
        self.recover_operations()

    @contextmanager
    def connect(self):
        db = sqlite3.connect(self.data_dir / 'library.sqlite3', timeout=30)
        db.row_factory = sqlite3.Row
        try:
            with db:
                yield db
        finally:
            db.close()

    def recover_operations(self):
        with self.connect() as db:
            for op in db.execute("SELECT * FROM operations WHERE state='pending'").fetchall():
                if not Path(op['source']).exists() and Path(op['destination']).exists():
                    trash = op['destination'] if op['kind'] == 'trash' else None
                    db.execute('UPDATE photos SET trash=?,missing=0 WHERE id=?', (trash, op['photo']))
                    db.execute("UPDATE operations SET state='done' WHERE id=?", (op['id'],))
                elif Path(op['source']).exists() and not Path(op['destination']).exists():
                    db.execute("UPDATE operations SET state='cancelled' WHERE id=?", (op['id'],))
                elif Path(op['source']).exists() and Path(op['destination']).exists():
                    # Crash between hard-link creation and source unlink: roll back only our link.
                    if os.path.samefile(op['source'], op['destination']):
                        Path(op['destination']).unlink()
                        db.execute("UPDATE operations SET state='cancelled' WHERE id=?", (op['id'],))

    def start_scan(self, folder):
        if not str(folder).strip():
            raise ValueError('Укажите папку с фотографиями')
        root = Path(folder).expanduser().resolve(strict=True)
        if not root.is_dir() or root == self.data_dir or self.data_dir in root.parents:
            raise ValueError('Укажите папку с фотографиями')
        with self.lock:
            if self.progress['running']:
                raise ValueError('Дождитесь завершения текущего сканирования')
            self.cancel.clear()
            self.progress = {'running': True, 'done': 0, 'errors': 0, 'message': 'Поиск фотографий…'}
            self.worker = threading.Thread(target=self.scan, args=(root,), daemon=True)
            self.worker.start()

    def close(self):
        self.cancel.set()
        if self.worker:
            self.worker.join()

    def scan(self, root):
        generation = uuid.uuid4().hex
        try:
            with self.connect() as db:
                for directory, dirs, files in os.walk(root, followlinks=False, onerror=self.scan_error):
                    dirs[:] = [d for d in dirs if d not in {'.cleaner_ultra-trash', '.photosort-trash', '.photosort-space-trash', '.photosort', '.git', '.cleaner_ultra'}
                               and not (Path(directory) / d).is_symlink()
                               and (Path(directory) / d).resolve() != self.data_dir]
                    for name in files:
                        if self.cancel.is_set():
                            break
                        path = Path(directory) / name
                        if path.suffix.lower() not in EXTENSIONS or path.is_symlink():
                            continue
                        try:
                            stat = path.stat()
                            old = db.execute('SELECT * FROM photos WHERE path=?', (str(path),)).fetchone()
                            if old and old['trash']:
                                continue
                            if old and old['mtime'] == stat.st_mtime_ns and old['size'] == stat.st_size:
                                db.execute('UPDATE photos SET seen=?,missing=0 WHERE id=?', (generation, old['id']))
                            else:
                                digest = fingerprint(path)
                                width, height, captured, dhash, sharpness = describe(path, self.thumbs / f'{digest}.jpg')
                                after = path.stat()
                                if (after.st_size, after.st_mtime_ns) != (stat.st_size, stat.st_mtime_ns):
                                    raise ValueError('Файл изменился во время чтения')
                                db.execute('''INSERT INTO photos
                                    (path,root,name,size,mtime,width,height,captured,hash,dhash,sharpness,seen)
                                    VALUES (?,?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(path) DO UPDATE SET
                                    root=excluded.root,name=excluded.name,size=excluded.size,mtime=excluded.mtime,
                                    width=excluded.width,height=excluded.height,captured=excluded.captured,
                                    hash=excluded.hash,dhash=excluded.dhash,sharpness=excluded.sharpness,
                                    seen=excluded.seen,missing=0''',
                                           (str(path), str(root), name, stat.st_size, stat.st_mtime_ns, width,
                                            height, captured, digest, dhash, sharpness, generation))
                            self.progress['done'] += 1
                            self.progress['message'] = name
                            if self.progress['done'] % 30 == 0:
                                db.commit()
                        except (OSError, ValueError, Image.DecompressionBombError) as error:
                            self.scan_error(error)
                    if self.cancel.is_set():
                        break
                if not self.cancel.is_set() and not self.progress['errors']:
                    # Only reconcile entries inside this root; scanning a child must not hide siblings.
                    for row in db.execute('SELECT id,path FROM photos WHERE trash IS NULL AND seen != ?', (generation,)).fetchall():
                        if Path(row['path']).is_relative_to(root):
                            db.execute('UPDATE photos SET missing=1 WHERE id=?', (row['id'],))
            self.progress['message'] = 'Группировка похожих кадров…'
            self.group()
            self.progress['message'] = 'Сканирование остановлено' if self.cancel.is_set() else 'Библиотека обновлена'
        except Exception as error:
            self.scan_error(error)
            self.progress['message'] = f'Ошибка сканирования: {error}'
        finally:
            self.progress['running'] = False

    def scan_error(self, error):
        self.progress['errors'] += 1
        self.progress['last_error'] = str(error)

    def group(self):
        with self.connect() as db:
            rows = db.execute('SELECT * FROM photos WHERE trash IS NULL AND missing=0 ORDER BY captured,id').fetchall()
            tree = BKTree()
            # A representative-based group avoids chains of progressively unrelated images.
            representatives = {}
            assignments = []
            previous_by_folder = {}
            for row in rows:
                value = int(row['dhash'], 16)
                similar = row['id']
                ratio = row['width'] / row['height']
                for candidate in tree.find(value):
                    if abs(representatives[candidate] - ratio) < 0.15:
                        similar = candidate
                        break
                if similar == row['id']:
                    tree.add(value, similar)
                    representatives[similar] = ratio
                folder = str(Path(row['path']).parent)
                previous, burst = previous_by_folder.get(folder, (None, None))
                if not (previous and row['captured'] and previous['captured']
                        and 0 <= row['captured'] - previous['captured'] <= 8
                        and Path(row['path']).parent == Path(previous['path']).parent):
                    burst = row['id']
                assignments.append((similar, burst, row['id']))
                previous_by_folder[folder] = (row, burst)
            db.executemany('UPDATE photos SET similar=?,burst=? WHERE id=?', assignments)

    def photo(self, photo_id):
        with self.connect() as db:
            row = db.execute('SELECT * FROM photos WHERE id=?', (photo_id,)).fetchone()
            if row is None:
                raise ValueError('Фотография не найдена')
            return dict(row)

    def listing(self, view='all', search='', offset=0, sort='date'):
        active = 'trash IS NULL AND missing=0'
        conditions = {'all': active, 'favorites': active + ' AND favorite=1', 'trash': 'trash IS NOT NULL'}
        group_field = {'duplicates': 'hash', 'similar': 'similar', 'bursts': 'burst'}.get(view)
        where = conditions.get(view, active)
        if group_field:
            where += f' AND {group_field} IN (SELECT {group_field} FROM photos WHERE {active} GROUP BY {group_field} HAVING COUNT(*)>1)'
        parameters = []
        if search:
            where += ' AND instr(lower(name),lower(?))>0'
            parameters.append(search)
        ordering = {'date': 'COALESCE(captured,mtime/1000000000) DESC', 'name': 'name COLLATE NOCASE',
                    'quality': 'sharpness DESC', 'size': 'size DESC'}.get(sort, 'id DESC')
        if group_field:
            ordering = f'{group_field},' + ordering
        with self.connect() as db:
            total = db.execute(f'SELECT COUNT(*) FROM photos WHERE {where}', parameters).fetchone()[0]
            rows = db.execute(f'SELECT * FROM photos WHERE {where} ORDER BY {ordering},id LIMIT 80 OFFSET ?',
                              parameters + [max(0, offset)]).fetchall()
            counts = {name: db.execute(f'SELECT COUNT(*) FROM photos WHERE {condition}').fetchone()[0]
                      for name, condition in conditions.items()}
            for name, field in [('duplicates', 'hash'), ('similar', 'similar'), ('bursts', 'burst')]:
                counts[name] = db.execute(f'''SELECT COUNT(*) FROM photos WHERE {active} AND {field} IN
                    (SELECT {field} FROM photos WHERE {active} GROUP BY {field} HAVING COUNT(*)>1)''').fetchone()[0]
            counts['bytes'] = db.execute(f'SELECT COALESCE(SUM(size),0) FROM photos WHERE {active}').fetchone()[0]
            return {'photos': [dict(row) for row in rows], 'total': total, 'counts': counts, 'progress': dict(self.progress)}

    def favorite(self, ids, value):
        with self.lock, self.connect() as db:
            db.executemany('UPDATE photos SET favorite=? WHERE id=?', [(int(bool(value)), i) for i in ids])

    def move(self, ids, restore=False):
        results = {'done': [], 'errors': []}
        with self.lock:
            if self.progress['running']:
                raise ValueError('Остановите сканирование перед перемещением файлов')
            for photo_id in ids:
                try:
                    photo = self.photo(photo_id)
                    if bool(photo['trash']) != restore:
                        continue
                    original = Path(photo['path'])
                    root = Path(photo['root']).resolve(strict=True)
                    if original.is_symlink() or not original.parent.resolve().is_relative_to(root):
                        raise ValueError('Исходный путь изменился или содержит внешнюю ссылку')
                    if restore:
                        source, target = Path(photo['trash']), original
                    else:
                        source = original
                        if fingerprint(source) != photo['hash']:
                            raise ValueError('Файл изменился. Сначала пересканируйте папку')
                        target = Path(photo['root']) / '.cleaner_ultra-trash' / f'{uuid.uuid4().hex}{source.suffix}'
                    if target.exists():
                        raise ValueError('Путь уже занят; существующий файл сохранён')
                    trash_dir = root / '.cleaner_ultra-trash'
                    if trash_dir.is_symlink() or (trash_dir.exists() and trash_dir.resolve() != trash_dir):
                        raise ValueError('Папка корзины не должна быть ссылкой')
                    target.parent.mkdir(parents=True, exist_ok=True)
                    with self.connect() as db:
                        operation = db.execute('INSERT INTO operations(photo,source,destination,kind) VALUES(?,?,?,?)',
                                               (photo_id, str(source), str(target), 'restore' if restore else 'trash')).lastrowid
                    # Exclusive hard-link creation is atomic and cannot overwrite another file.
                    # Trash lives on the same volume. If links are unsupported, fail safely.
                    os.link(source, target)
                    try:
                        source.unlink()
                    except OSError:
                        target.unlink()
                        raise
                    with self.connect() as db:
                        db.execute('UPDATE photos SET trash=?,missing=0 WHERE id=?',
                                   (None if restore else str(target), photo_id))
                        db.execute("UPDATE operations SET state='done' WHERE id=?", (operation,))
                    results['done'].append(photo_id)
                except (OSError, ValueError, sqlite3.Error) as error:
                    results['errors'].append({'id': photo_id, 'message': str(error)})
            self.group()
        return results
