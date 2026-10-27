"""Native Windows host, with its own single-instance data directory."""
import ctypes
import logging
import os
import sys
import threading
from pathlib import Path

from .library import Library
from .server import make_server


def main():
    import webview

    mutex = None
    if sys.platform == 'win32':
        kernel = ctypes.windll.kernel32
        kernel.CreateMutexW.restype = ctypes.c_void_p
        mutex = kernel.CreateMutexW(None, False, 'Local\\CleanerUltraDesktop')
        if kernel.GetLastError() == 183:
            ctypes.windll.user32.MessageBoxW(None, 'Cleaner Ultra уже запущен.', 'Cleaner Ultra', 0x40)
            return
    data = Path(os.environ.get('LOCALAPPDATA', str(Path.home() / '.local' / 'share'))) / 'CleanerUltra'
    data.mkdir(parents=True, exist_ok=True)
    logging.basicConfig(filename=data / 'cleaner_ultra.log', level=logging.WARNING)
    library = Library(data)
    window = None

    def pick_folder():
        selected = window.create_file_dialog(webview.FileDialog.FOLDER)
        return selected[0] if selected else ''

    server = make_server(library, 0, folder_picker=pick_folder)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        window = webview.create_window('Cleaner Ultra', f'http://127.0.0.1:{server.server_port}',
                              width=1440, height=940, min_size=(860, 640), background_color='#fafafa')
        webview.start()
    except Exception:
        logging.exception('Desktop startup failed')
        if sys.platform == 'win32':
            ctypes.windll.user32.MessageBoxW(None,
                'Не удалось открыть Cleaner Ultra. Установите Microsoft Edge WebView2 Runtime. Подробности: %LOCALAPPDATA%\\CleanerUltra\\cleaner_ultra.log',
                'Cleaner Ultra', 0x10)
        raise
    finally:
        server.shutdown()
        library.close()
        server.server_close()
        if mutex:
            ctypes.windll.kernel32.CloseHandle(ctypes.c_void_p(mutex))


if __name__ == '__main__':
    main()
