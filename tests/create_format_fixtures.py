"""Generate a valid, synthetic Bayer DNG; no user photographs."""
from pathlib import Path
import numpy as np
import tifffile

root = Path(__file__).resolve().parent.parent / 'build' / 'format-fixtures'
root.mkdir(parents=True, exist_ok=True)
raw = np.fromfunction(lambda y, x: 800 + x * 80 + y * 40, (128, 128), dtype=int).astype(np.uint16)
tags = [
    (50706, 'B', 4, (1, 4, 0, 0), False),
    (50707, 'B', 4, (1, 1, 0, 0), False),
    (50708, 's', 0, 'CleanerUltra Synthetic Camera', False),
    (33421, 'H', 2, (2, 2), False),
    (33422, 'B', 4, (0, 1, 1, 2), False),
    (50710, 'B', 3, (0, 1, 2), False),
    (50711, 'H', 1, 1, False),
    (50714, 'H', 1, 0, False),
    (50717, 'I', 1, 65535, False),
    (50721, '2i', 9, (1,1,0,1,0,1,0,1,1,1,0,1,0,1,0,1,1,1), False),
    (50728, '2I', 3, (1,1,1,1,1,1), False),
    (50778, 'H', 1, 21, False),
]
tifffile.imwrite(root / 'TEST_bayer.dng', raw, photometric=32803, extratags=tags, metadata=None)
print(root)
