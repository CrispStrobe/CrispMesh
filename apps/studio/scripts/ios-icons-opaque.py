"""Rewrites src-tauri/icons/ios/*.png without an alpha channel.

`tauri icon` writes RGBA files, and App Store Connect refuses an app icon that has an alpha
channel. The icons come from two sources in apps/desktop/src-tauri/icons: source.png (full
bleed; iOS and Android mask it themselves, Windows tiles) and source-desktop.png (the rounded
square with margin that macOS and Windows show as it is):

    npx tauri icon ../desktop/src-tauri/icons/source-desktop.png -o /tmp/desktop
    npx tauri icon ../desktop/src-tauri/icons/source.png -o /tmp/mobile
    # icon.icns, icon.ico, icon.png and the NxN PNGs from /tmp/desktop; ios/, android/,
    # Square*Logo.png and StoreLogo.png from /tmp/mobile (both apps), then:
    python scripts/ios-icons-opaque.py

Needs Pillow.
"""

from pathlib import Path

from PIL import Image

BACKGROUND = (0x15, 0x26, 0x2F)
folder = Path(__file__).resolve().parent.parent / "src-tauri" / "icons" / "ios"
changed = 0
for path in sorted(folder.glob("*.png")):
    image = Image.open(path)
    if image.mode == "RGB":
        continue
    flat = Image.new("RGB", image.size, BACKGROUND)
    flat.paste(image.convert("RGBA"), mask=image.convert("RGBA").split()[3])
    flat.save(path, optimize=True)
    changed += 1
print(f"{changed} icons rewritten without alpha in {folder}")
