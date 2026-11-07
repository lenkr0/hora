"""Programmsymbol von Hora erzeugen: crates/app/assets/hora.ico und hora.png.

Große Größen: Raute in Calinos Akzentfarbe auf warmem Papier-Ton.
Kleine Größen (Infobereich, 16–24 px): nur die Raute, damit sie auf hellen
und dunklen Taskleisten gleich gut lesbar ist.

    python tools/make-icon.py
"""
from pathlib import Path

from PIL import Image, ImageDraw

ACCENT = (176, 125, 79, 255)       # #b07d4f
ACCENT_DARK = (120, 85, 54, 255)   # #785536
PAPER_TOP = (250, 248, 243, 255)   # #faf8f3
PAPER_BOTTOM = (240, 235, 225, 255)  # #f0ebe1
EDGE = (44, 40, 35, 40)
SCALE = 8


def diamond(draw, cx, cy, half, radius, fill):
    """Abgerundete Raute: Quadrat mit runden Ecken, um 45° gedreht."""
    size = int(half * 2 * SCALE)
    tile = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    ImageDraw.Draw(tile).rounded_rectangle((0, 0, size - 1, size - 1), radius=int(radius * SCALE), fill=fill)
    tile = tile.rotate(45, resample=Image.BICUBIC, expand=True)
    return tile, (int(cx * SCALE - tile.width / 2), int(cy * SCALE - tile.height / 2))


def render(px, small):
    big = Image.new("RGBA", (px * SCALE, px * SCALE), (0, 0, 0, 0))
    if not small:
        # Papier mit leichtem Verlauf, abgerundet wie eine Kachel
        paper = Image.new("RGBA", big.size)
        top, bottom = PAPER_TOP, PAPER_BOTTOM
        for y in range(big.height):
            t = y / big.height
            row = tuple(int(top[i] + (bottom[i] - top[i]) * t) for i in range(4))
            ImageDraw.Draw(paper).line((0, y, big.width, y), fill=row)
        mask = Image.new("L", big.size, 0)
        inset = px * SCALE * 0.06
        ImageDraw.Draw(mask).rounded_rectangle(
            (inset, inset, big.width - inset, big.height - inset), radius=px * SCALE * 0.22, fill=255
        )
        big.paste(paper, (0, 0), mask)
        ImageDraw.Draw(big).rounded_rectangle(
            (inset, inset, big.width - inset, big.height - inset),
            radius=px * SCALE * 0.22,
            outline=EDGE,
            width=max(1, SCALE // 2),
        )
        half, radius = px * 0.25, px * 0.07
    else:
        half, radius = px * 0.34, px * 0.09
    tile, pos = diamond(None, px / 2, px / 2, half, radius, ACCENT)
    big.alpha_composite(tile, pos)
    # Kleiner dunkler Kern wie in der Kopfzeile
    core, pos = diamond(None, px / 2, px / 2, half * 0.36, radius * 0.5, ACCENT_DARK)
    big.alpha_composite(core, pos)
    return big.resize((px, px), Image.LANCZOS)


def main():
    out = Path(__file__).resolve().parent.parent / "crates" / "app" / "assets"
    out.mkdir(parents=True, exist_ok=True)
    sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256]
    images = [render(s, small=s <= 24) for s in sizes]
    images[-1].save(out / "hora.ico", sizes=[(s, s) for s in sizes], append_images=images[:-1])
    images[-1].save(out / "hora.png")
    print(f"geschrieben: {out / 'hora.ico'}, {out / 'hora.png'}")


if __name__ == "__main__":
    main()
