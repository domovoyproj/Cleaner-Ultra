import json
import os
import shutil
import tempfile
import threading
import unittest
import urllib.error
import urllib.request
from pathlib import Path

from PIL import Image

from cleaner_ultra.library import BKTree, Library, fingerprint
from cleaner_ultra.server import make_server


class LibraryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name) / 'photos'
        self.root.mkdir()
        self.library = Library(Path(self.temp.name) / 'index')

    def tearDown(self):
        self.library.close()
        self.temp.cleanup()

    def photo(self, name='one.jpg', color='red', date=None):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        image = Image.new('RGB', (80, 60), color)
        exif = Image.Exif()
        if date:
            exif[36867] = date
        image.save(path, exif=exif)
        return path

    def scan(self):
        self.library.scan(self.root)
        return self.library.listing()['photos']

    def test_exact_duplicates_and_incremental_scan(self):
        source = self.photo()
        shutil.copyfile(source, self.root / 'copy.jpg')
        photos = self.scan()
        self.assertEqual(self.library.listing('duplicates')['total'], 2)
        before = (self.library.thumbs / f"{photos[0]['hash']}.jpg").stat().st_mtime_ns
        self.scan()
        self.assertEqual((self.library.thumbs / f"{photos[0]['hash']}.jpg").stat().st_mtime_ns, before)
        self.assertEqual(self.library.listing()['total'], 2)

    def test_trash_restore_and_persistence(self):
        path = self.photo()
        digest = fingerprint(path)
        photo = self.scan()[0]
        self.library.favorite([photo['id']], True)
        result = self.library.move([photo['id']])
        self.assertEqual(result['errors'], [])
        self.assertFalse(path.exists())
        self.assertEqual(self.library.listing('trash')['total'], 1)
        self.library = Library(self.library.data_dir)
        self.assertTrue(self.library.photo(photo['id'])['favorite'])
        self.assertEqual(self.library.move([photo['id']], restore=True)['errors'], [])
        self.assertEqual(fingerprint(path), digest)

    def test_restore_never_overwrites_conflict(self):
        path = self.photo()
        photo = self.scan()[0]
        self.library.move([photo['id']])
        path.write_bytes(b'new content')
        result = self.library.move([photo['id']], restore=True)
        self.assertEqual(len(result['errors']), 1)
        self.assertEqual(path.read_bytes(), b'new content')
        self.assertTrue(Path(self.library.photo(photo['id'])['trash']).exists())

    def test_modified_file_is_not_moved(self):
        path = self.photo()
        photo = self.scan()[0]
        path.write_bytes(b'changed')
        self.assertTrue(self.library.move([photo['id']])['errors'])
        self.assertEqual(path.read_bytes(), b'changed')

    def test_corrupt_file_and_trash_are_skipped(self):
        self.photo()
        self.photo('.cleaner_ultra-trash/ignored.jpg')
        (self.root / 'broken.jpg').write_bytes(b'broken')
        self.scan()
        self.assertEqual(self.library.listing()['total'], 1)
        self.assertEqual(self.library.progress['errors'], 1)

    def test_burst_across_interleaved_folders(self):
        self.photo('a/1.jpg', date='2026:10:01 10:00:00')
        self.photo('b/1.jpg', date='2026:10:01 10:00:01')
        self.photo('a/2.jpg', date='2026:10:01 10:00:02')
        self.photo('no-exif.jpg')
        self.scan()
        self.assertEqual(self.library.listing('bursts')['total'], 2)

    def test_child_scan_does_not_hide_siblings(self):
        self.photo('a/1.jpg')
        self.photo('b/2.jpg')
        self.scan()
        self.library.scan(self.root / 'a')
        self.assertEqual(self.library.listing()['total'], 2)
        (self.root / 'a/1.jpg').unlink()
        self.library.scan(self.root / 'a')
        self.assertEqual(self.library.listing()['total'], 1)

    def test_cancel_does_not_mark_missing(self):
        self.photo()
        self.scan()
        self.library.cancel.set()
        self.scan()
        self.assertEqual(self.library.listing()['total'], 1)

    def test_operation_recovery_after_move(self):
        path = self.photo()
        photo = self.scan()[0]
        destination = self.root / '.cleaner_ultra-trash' / 'recovery.jpg'
        destination.parent.mkdir()
        with self.library.connect() as db:
            db.execute('INSERT INTO operations(photo,source,destination,kind) VALUES(?,?,?,?)',
                       (photo['id'], str(path), str(destination), 'trash'))
        path.rename(destination)
        restarted = Library(self.library.data_dir)
        self.assertEqual(restarted.photo(photo['id'])['trash'], str(destination))
        self.assertEqual(restarted.move([photo['id']], restore=True)['errors'], [])

    def test_operation_recovery_between_link_and_unlink(self):
        path = self.photo()
        photo = self.scan()[0]
        destination = self.root / 'pending.jpg'
        with self.library.connect() as db:
            db.execute('INSERT INTO operations(photo,source,destination,kind) VALUES(?,?,?,?)',
                       (photo['id'], str(path), str(destination), 'trash'))
        os.link(path, destination)
        Library(self.library.data_dir)
        self.assertTrue(path.exists())
        self.assertFalse(destination.exists())

    def test_metric_search(self):
        tree = BKTree()
        for value in [0, 7, 15, 255, 65535]:
            tree.add(value, value)
        self.assertEqual(set(tree.find(0, 3)), {0, 7})

    def test_scan_blocks_moves(self):
        self.photo()
        photo = self.scan()[0]
        self.library.progress['running'] = True
        with self.assertRaises(ValueError):
            self.library.move([photo['id']])
        self.library.progress['running'] = False

    def test_api_session_and_csrf(self):
        server = make_server(self.library, 0)
        worker = threading.Thread(target=server.serve_forever, daemon=True)
        worker.start()
        url = f'http://127.0.0.1:{server.server_port}'
        try:
            with urllib.request.urlopen(url + '/api/session') as response:
                token = json.load(response)['token']
            request = urllib.request.Request(url + '/api/favorite', data=b'{"ids": []}', method='POST')
            with self.assertRaises(urllib.error.HTTPError) as error:
                urllib.request.urlopen(request)
            self.assertEqual(error.exception.code, 403)
            request.add_header('X-CleanerUltra-Token', token)
            with urllib.request.urlopen(request) as response:
                self.assertTrue(json.load(response)['ok'])
            request = urllib.request.Request(url + '/', headers={'Host': 'evil.example'})
            with self.assertRaises(urllib.error.HTTPError) as error:
                urllib.request.urlopen(request)
            self.assertEqual(error.exception.code, 403)
        finally:
            server.shutdown()
            server.server_close()


if __name__ == '__main__':
    unittest.main()
