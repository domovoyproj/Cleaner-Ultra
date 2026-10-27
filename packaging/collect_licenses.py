"""Include installed dependency notices with redistributable builds."""
import importlib.metadata
import shutil
import sys
from pathlib import Path

destination = Path(__file__).resolve().parent.parent / 'dist' / 'CleanerUltra' / 'licenses'
destination.mkdir(parents=True, exist_ok=True)
packages = ['Pillow', 'pywebview', 'pythonnet', 'clr_loader', 'cffi', 'pycparser',
            'bottle', 'proxy_tools', 'typing_extensions', 'setuptools', 'numpy']
manifest = []
for name in packages:
    try:
        distribution = importlib.metadata.distribution(name)
    except importlib.metadata.PackageNotFoundError:
        continue
    manifest.append(f'{name} {distribution.version}')
    for relative in distribution.files or []:
        if any(word in relative.name.lower() for word in ['license', 'copying', 'notice']):
            source = Path(distribution.locate_file(relative))
            if source.is_file():
                target = destination / name / str(relative).replace('..', '_')
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(source, target)
python_license = Path(sys.base_prefix) / 'LICENSE.txt'
if python_license.exists():
    shutil.copyfile(python_license, destination / 'Python-LICENSE.txt')
(destination / 'COMPONENTS.txt').write_text('\n'.join(manifest), encoding='utf-8')
print(f'Collected dependency notices for {len(manifest)} components')
