"""Generate the app's geometric icon; no external assets or fonts."""
from pathlib import Path
from PIL import Image, ImageDraw

image = Image.new('RGBA', (256, 256))
draw = ImageDraw.Draw(image)
draw.rounded_rectangle((8, 8, 248, 248), radius=55, fill='#242424')
draw.polygon([(128, 45), (210, 128), (128, 211), (46, 128)], fill='white')
draw.polygon([(128, 75), (179, 128), (128, 181), (77, 128)], fill='#242424')
draw.polygon([(128, 107), (149, 128), (128, 149), (107, 128)], fill='white')
image.save(Path(__file__).with_name('cleaner_ultra.ico'), sizes=[(16, 16), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
