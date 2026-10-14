"""Draws the OpenSheet icon as SVG: a sheet of cells on an indigo tile, one cell
selected with its fill handle. Coordinates are those of the whole tile
(0..1024); the macOS variant insets it on Apple's 824-point grid.

    python3 icon.py flat|small|mac|mac-small|layer-sheet|layer-selection > out.svg
"""
import sys

TOP, BOTTOM = "#8b8cff", "#3b2fc9"
SELECTION = "#22e1ff"
SELECTED_FILL = "#dff9ff"


def svg(body):
    return f'<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024">{body}</svg>\n'


def tile(rx):
    return (f'<defs><linearGradient id="bg" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="{TOP}"/>'
            f'<stop offset="1" stop-color="{BOTTOM}"/></linearGradient></defs>'
            f'<rect width="1024" height="1024" rx="{rx}" fill="url(#bg)"/>')


def sheet():
    """Column headers, row headers (translucent) and six cells."""
    xs = [(184, 104), (304, 260), (580, 260)]
    ys = [(200, 104), (320, 157), (493, 157), (666, 158)]
    cells = []
    for r, (y, h) in enumerate(ys):
        for c, (x, w) in enumerate(xs):
            if r == 0 and c == 0:
                continue
            opacity = 0.62 if r == 0 or c == 0 else 1.0
            cells.append(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="22" fill="#ffffff" fill-opacity="{opacity}"/>')
    return "".join(cells)


def selection():
    x, y, w, h, t = 580, 493, 260, 157, 26
    return (f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="22" fill="{SELECTED_FILL}"/>'
            f'<rect x="{x - t / 2}" y="{y - t / 2}" width="{w + t}" height="{h + t}" rx="32" fill="none" stroke="{SELECTION}" stroke-width="{t}"/>'
            f'<rect x="{x + w - 16}" y="{y + h - 16}" width="56" height="56" rx="10" fill="{SELECTION}" stroke="#ffffff" stroke-width="12"/>')


def small():
    """16 to 32 pixels: a header row, two rows of two cells, a thick selection."""
    xs = [(176, 324), (524, 324)]
    ys = [(176, 150), (350, 230), (604, 244)]
    cells = []
    for r, (y, h) in enumerate(ys):
        for c, (x, w) in enumerate(xs):
            fill = SELECTED_FILL if (r, c) == (2, 1) else "#ffffff"
            opacity = 0.62 if r == 0 else 1.0
            cells.append(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="34" fill="{fill}" fill-opacity="{opacity}"/>')
    x, y, w, h, t = 524, 604, 324, 244, 52
    cells.append(f'<rect x="{x - t / 2 + 6}" y="{y - t / 2 + 6}" width="{w + t - 12}" height="{h + t - 12}" rx="50" '
                 f'fill="none" stroke="{SELECTION}" stroke-width="{t}"/>')
    return "".join(cells)


def mac(body, shadow=True):
    """Apple's grid for icons before macOS 26: an 824-point tile with a soft
    shadow (left out at 32 pixels and below, where it only blurs the edge)."""
    shadow = shadow and ('<defs><filter id="sh" x="-20%" y="-20%" width="140%" height="140%"><feGaussianBlur stdDeviation="14"/></filter></defs>'
              '<rect x="100" y="112" width="824" height="824" rx="185" fill="#000" fill-opacity="0.28" filter="url(#sh)"/>')
    return (shadow or "") + f'<g transform="translate(100 100) scale({824 / 1024})">{body}</g>'


def padded(body):
    """Windows and Linux: the tile with a small margin, as their icons have."""
    return f'<g transform="translate(48 48) scale({928 / 1024})">{body}</g>'


VARIANTS = {
    "flat": lambda: svg(padded(tile(230) + sheet() + selection())),
    "small": lambda: svg(tile(200) + small()),
    "mac": lambda: svg(mac(tile(230) + sheet() + selection())),
    "mac-small": lambda: svg(mac(tile(230) + small(), shadow=False)),
    "layer-sheet": lambda: svg(sheet()),
    "layer-selection": lambda: svg(selection()),
}

if __name__ == "__main__":
    sys.stdout.write(VARIANTS[sys.argv[1]]())
