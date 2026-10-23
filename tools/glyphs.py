#!/usr/bin/env python3
"""Writes nori's Iconoir glyphs: Iconoir.kt, and the XML drawables the widgets, shortcuts and car use.

    python3 tools/glyphs.py <iconoir>/icons

<iconoir>/icons is the icons folder of the Iconoir package (MIT; `npm pack iconoir`), with regular/ and solid/.
Add a glyph to GLYPHS and run this again; nothing else lists them.
"""
import re
import sys
from pathlib import Path
from xml.etree import ElementTree as ET

ROOT = Path(__file__).resolve().parent.parent
KOTLIN = ROOT / "app/src/main/kotlin/dev/nori/music/app/ui/Iconoir.kt"
CORE = ROOT / "core/src/main/res/drawable"
APP = ROOT / "app/src/main/res/drawable"

# Kotlin name: (Iconoir icon, style, flips in right-to-left layouts)
GLYPHS = {
    "HomeSimple": ("home-simple", "regular", False),
    "AlbumList": ("album-list", "regular", False),
    "Settings": ("settings", "regular", False),
    "Search": ("search", "regular", False),
    "ArrowLeft": ("arrow-left", "regular", True),
    "NavArrowRight": ("nav-arrow-right", "regular", True),
    "NavArrowDown": ("nav-arrow-down", "regular", False),
    "NavArrowUp": ("nav-arrow-up", "regular", False),
    "MoreHoriz": ("more-horiz", "regular", False),
    "Xmark": ("xmark", "regular", False),
    "Plus": ("plus", "regular", False),
    "Check": ("check", "regular", False),
    "Menu": ("menu", "regular", False),
    "PlaySolid": ("play", "solid", False),
    "PauseSolid": ("pause", "solid", False),
    "RewindSolid": ("rewind", "solid", False),
    "ForwardSolid": ("forward", "solid", False),
    "Shuffle": ("shuffle", "regular", False),
    "Repeat": ("repeat", "regular", False),
    "RepeatOnce": ("repeat-once", "regular", False),
    "EmojiSingRightNote": ("emoji-sing-right-note", "regular", False),
    "Playlist": ("playlist", "regular", True),
    "PlaylistPlay": ("playlist-play", "regular", True),
    "PlusCircle": ("plus-circle", "regular", False),
    "MinusCircle": ("minus-circle", "regular", False),
    "CompactDisc": ("compact-disc", "regular", False),
    "User": ("user", "regular", False),
    "FolderPlus": ("folder-plus", "regular", False),
    "HalfMoon": ("half-moon", "regular", False),
    "AntennaSignal": ("antenna-signal", "regular", False),
    "Sparks": ("sparks", "regular", False),
    "Prohibition": ("prohibition", "regular", False),
    "ShareIos": ("share-ios", "regular", False),
    "InfoCircle": ("info-circle", "regular", False),
    "Community": ("community", "regular", False),
    "Trash": ("trash", "regular", False),
    "EditPencil": ("edit-pencil", "regular", False),
    "Refresh": ("refresh", "regular", False),
    "Copy": ("copy", "regular", False),
    "Key": ("key", "regular", False),
    "UserPlus": ("user-plus", "regular", False),
    "LogOut": ("log-out", "regular", True),
    "ShareAndroid": ("share-android", "regular", False),
    "Download": ("download", "regular", False),
    "CheckCircle": ("check-circle", "regular", False),
    "DataTransferDown": ("data-transfer-down", "regular", False),
    "CloudDownload": ("cloud-download", "regular", False),
    "WarningCircle": ("warning-circle", "regular", False),
    "MusicNote": ("music-note", "regular", False),
    "MusicDoubleNote": ("music-double-note", "regular", False),
    "ClockRotateRight": ("clock-rotate-right", "regular", False),
    "ViewGrid": ("view-grid", "regular", False),
    "Calendar": ("calendar", "regular", False),
    "Folder": ("folder", "regular", False),
    "Server": ("server", "regular", False),
    "Play": ("play", "regular", False),
    "ControlSlider": ("control-slider", "regular", False),
    "Palette": ("palette", "regular", False),
    "DashboardSpeed": ("dashboard-speed", "regular", False),
    "MediaImage": ("media-image", "regular", False),
    "SmartphoneDevice": ("smartphone-device", "regular", False),
    "Headset": ("headset", "regular", False),
    "BluetoothTag": ("bluetooth-tag", "regular", False),
    "Airplay": ("airplay", "regular", False),
    "Computer": ("computer", "regular", False),
    "Terminal": ("terminal", "regular", False),
    "StatsUpSquare": ("stats-up-square", "regular", False),
    "SoundHigh": ("sound-high", "regular", False),
    "SoundHighSolid": ("sound-high", "solid", False),
    "SoundLow": ("sound-low", "regular", False),
    "SoundOff": ("sound-off", "regular", False),
}

# XML drawable: (directory, glyph, shortcut?)
DRAWABLES = {
    "car_add_to_queue": ("core", "PlusCircle", False),
    "car_albums": ("core", "CompactDisc", False),
    "car_artists": ("core", "User", False),
    "car_download": ("core", "Download", False),
    "car_genres": ("core", "ViewGrid", False),
    "car_home": ("core", "HomeSimple", False),
    "car_library": ("core", "AlbumList", False),
    "car_most": ("core", "StatsUpSquare", False),
    "car_new": ("core", "Sparks", False),
    "car_play": ("core", "PlaySolid", False),
    "car_play_next": ("core", "PlaylistPlay", False),
    "car_playlists": ("core", "Playlist", False),
    "car_random": ("core", "Shuffle", False),
    "car_recent": ("core", "ClockRotateRight", False),
    "car_shuffle": ("core", "Shuffle", False),
    "widget_forward": ("app", "ForwardSolid", False),
    "widget_note": ("app", "MusicNote", False),
    "widget_pause": ("app", "PauseSolid", False),
    "widget_play": ("app", "PlaySolid", False),
    "widget_rewind": ("app", "RewindSolid", False),
    "widget_shuffle": ("app", "Shuffle", False),
    "widget_sparkle": ("app", "Sparks", False),
    "shortcut_album_glyph": ("app", "CompactDisc", True),
    "shortcut_search_glyph": ("app", "Search", True),
    "shortcut_shuffle_glyph": ("app", "Shuffle", True),
}

NS = "{http://www.w3.org/2000/svg}"

# Drawn to the seal's height (core/src/main/res/drawable/seal_outline.xml, a square frame, 3 to 21) and
# centred on it, line width unchanged, so a device glyph beside the seal is as tall and as heavy. A glyph too
# wide for that stops at [WIDEST], its line still inside the box.
FITTED = {"SmartphoneDevice", "Headset", "BluetoothTag", "Airplay", "Computer", "Terminal"}
FRAME = (3.0, 3.0, 21.0, 21.0)
WIDEST = 22.5

ARGS = {"M": 2, "L": 2, "T": 2, "H": 1, "V": 1, "C": 6, "S": 4, "Q": 4, "A": 7, "Z": 0}


def commands(d):
    tokens = re.findall(r"[MLHVCSQTAZmlhvcsqtaz]|-?(?:\d+\.?\d*|\.\d+)(?:[eE][-+]?\d+)?", d)
    out, i = [], 0
    while i < len(tokens):
        c = tokens[i]; i += 1
        n = ARGS[c.upper()]
        if n == 0:
            out.append((c, [])); continue
        first = True
        while i < len(tokens) and not tokens[i].isalpha():
            out.append((c if first or c.upper() != "M" else ("L" if c == "M" else "l"), [float(x) for x in tokens[i:i + n]]))
            i += n; first = False
    return out


def points(cmds):
    """Every point a path passes through or pulls toward, absolute."""
    x = y = sx = sy = 0.0
    for c, a in cmds:
        rel = c.islower(); C = c.upper()
        if C == "Z":
            x, y = sx, sy; continue
        if C == "H":
            x = x + a[0] if rel else a[0]; yield x, y; continue
        if C == "V":
            y = y + a[0] if rel else a[0]; yield x, y; continue
        pairs = [(a[5], a[6])] if C == "A" else list(zip(a[0::2], a[1::2]))
        for px, py in pairs:
            yield (x + px, y + py) if rel else (px, py)
        x, y = (x + pairs[-1][0], y + pairs[-1][1]) if rel else pairs[-1]
        if C == "M":
            sx, sy = x, y


def fitted(ds):
    """[ds] scaled about their middle to [FRAME]'s height, or to [WIDEST] if that comes first, and centred on it."""
    every = [pt for d in ds for pt in points(commands(d))]
    x0, y0 = min(p[0] for p in every), min(p[1] for p in every)
    x1, y1 = max(p[0] for p in every), max(p[1] for p in every)
    k = min(WIDEST / max(x1 - x0, 1e-6), (FRAME[3] - FRAME[1]) / max(y1 - y0, 1e-6))
    mx, my = (x0 + x1) / 2, (y0 + y1) / 2
    cx, cy = (FRAME[0] + FRAME[2]) / 2, (FRAME[1] + FRAME[3]) / 2
    X = lambda v: f"{cx + (v - mx) * k:.4g}"
    Y = lambda v: f"{cy + (v - my) * k:.4g}"
    R = lambda v: f"{v * k:.4g}"
    out = []
    for d in ds:
        parts_ = []
        for c, a in commands(d):
            rel = c.islower(); C = c.upper()
            if C == "Z":
                parts_.append(c); continue
            if C == "H":
                parts_.append(c + (R(a[0]) if rel else X(a[0]))); continue
            if C == "V":
                parts_.append(c + (R(a[0]) if rel else Y(a[0]))); continue
            if C == "A":
                end = (R(a[5]), R(a[6])) if rel else (X(a[5]), Y(a[6]))
                parts_.append(c + " ".join([R(a[0]), R(a[1]), f"{a[2]:g}", f"{a[3]:g}", f"{a[4]:g}", *end])); continue
            xy = [(R(px), R(py)) if rel else (X(px), Y(py)) for px, py in zip(a[0::2], a[1::2])]
            parts_.append(c + " ".join(v for pair in xy for v in pair))
        out.append("".join(parts_))
    return out


def parts(svg: Path):
    root = ET.parse(svg).getroot()
    inherited = float(root.get("stroke-width", 1.5))
    out = []
    for el in root.iter():
        tag = el.tag.replace(NS, "")
        if tag in ("svg", "defs", "clipPath", "g"):
            if tag in ("g", "clipPath") or el.get("transform"):
                raise SystemExit(f"{svg}: <{tag}> is not handled")
            continue
        if el.get("transform") or el.get("stroke-dasharray"):
            raise SystemExit(f"{svg}: transform and dashes are not handled")
        if tag == "path":
            d = el.get("d")
        elif tag == "circle":
            cx, cy, r = (float(el.get(k)) for k in ("cx", "cy", "r"))
            d = f"M{cx - r:g} {cy:g}a{r:g} {r:g} 0 1 0 {2 * r:g} 0a{r:g} {r:g} 0 1 0 {-2 * r:g} 0Z"
        elif tag == "rect":
            x, y, w, h = (float(el.get(k)) for k in ("x", "y", "width", "height"))
            r = float(el.get("rx", 0))
            d = (f"M{x + r:g} {y:g}H{x + w - r:g}a{r:g} {r:g} 0 0 1 {r:g} {r:g}V{y + h - r:g}a{r:g} {r:g} 0 0 1 {-r:g} {r:g}"
                 f"H{x + r:g}a{r:g} {r:g} 0 0 1 {-r:g} {-r:g}V{y + r:g}a{r:g} {r:g} 0 0 1 {r:g} {-r:g}Z")
        else:
            raise SystemExit(f"{svg}: <{tag}> is not handled")
        fill = el.get("fill", "none") != "none"
        stroke = el.get("stroke") is not None
        if {el.get("stroke-linecap", "butt"), el.get("stroke-linejoin", "miter")} - {"round", "butt", "miter"}:
            raise SystemExit(f"{svg}: only round and plain ends are handled")
        out.append(dict(d=d, fill=fill, stroke=stroke, width=float(el.get("stroke-width", inherited)),
                        cap=el.get("stroke-linecap") == "round", join=el.get("stroke-linejoin") == "round",
                        even=el.get("fill-rule") == "evenodd"))
    return out


def kotlin_part(p):
    if p["stroke"] and not p["fill"] and p["width"] == 1.5 and p["cap"] and p["join"] and not p["even"]:
        return f'"{p["d"]}"'
    args = [f'"{p["d"]}"']
    if p["fill"]:
        args.append("fill = true")
    if p["even"]:
        args.append("evenOdd = true")
    if not p["stroke"]:
        args.append("stroke = false")
    if p["width"] != 1.5:
        args.append(f'width = {p["width"]:g}f')
    if p["stroke"] and not p["cap"]:
        args.append("cap = false")
    if p["stroke"] and not p["join"]:
        args.append("join = false")
    return f'Part({", ".join(args)})'


def xml_part(p, colour):
    a = [f'android:pathData="{p["d"]}"']
    if p["fill"]:
        a.append(f'android:fillColor="{colour}"')
    if p["even"]:
        a.append('android:fillType="evenOdd"')
    if p["stroke"]:
        a += [f'android:strokeColor="{colour}"', f'android:strokeWidth="{p["width"]:g}"']
        if p["cap"]:
            a.append('android:strokeLineCap="round"')
        if p["join"]:
            a.append('android:strokeLineJoin="round"')
    return "<path " + " ".join(a) + " />"


def main(icons: Path):
    shapes = {name: parts(icons / style / f"{icon}.svg") for name, (icon, style, _) in GLYPHS.items()}
    for name in FITTED:
        for p, d in zip(shapes[name], fitted([p["d"] for p in shapes[name]])):
            p["d"] = d
    lines = []
    for name, (icon, style, flip) in GLYPHS.items():
        ps = [kotlin_part(p) for p in shapes[name]]
        make = "glyph" if all(q.startswith('"') for q in ps) else "parts"
        if make == "parts":
            ps = [f"Part({q})" if q.startswith('"') else q for q in ps]
        body = ",\n        ".join(ps)
        lines.append(f'    /** {icon}{" (solid)" if style == "solid" else ""} */\n    val {name} by {make}({"true" if flip else "false"},\n        {body},\n    )')
    KOTLIN.write_text(KOTLIN_HEAD + "\n\n".join(lines) + "\n}\n")

    for file, (folder, glyph, shortcut) in DRAWABLES.items():
        path = (CORE if folder == "core" else APP) / f"{file}.xml"
        old = path.read_text()
        comment = re.search(r"<!--(.*?)-->", old, re.S).group(1).strip()
        role = re.sub(r"^Icons\.[\w.]+\s*[:,]?\s*|^Iconoir [\w-]+:\s*", "", comment)
        icon = GLYPHS[glyph][0]
        if shortcut:
            head = f"<!-- {comment} -->"
            ps = "\n        ".join(xml_part(p, "#FF1A1A1A") for p in shapes[glyph])
            body = (f'    android:width="108dp" android:height="108dp"\n    android:viewportWidth="108" android:viewportHeight="108">\n'
                    f'    <group android:scaleX="1.5" android:scaleY="1.5" android:translateX="36" android:translateY="36">\n        {ps}\n    </group>')
        else:
            head = f"<!-- Iconoir {icon}: {role} -->" if not role.startswith("The widgets' glyphs") else f"<!-- {role.replace('Icons.Filled.PlayArrow', 'Iconoir play')} -->"
            mirror = ' android:autoMirrored="true"' if GLYPHS[glyph][2] else ""
            ps = "\n    ".join(xml_part(p, "#FFFFFFFF") for p in shapes[glyph])
            body = f'    android:width="24dp" android:height="24dp" android:viewportWidth="24" android:viewportHeight="24"{mirror}>\n    {ps}'
        path.write_text(f'<?xml version="1.0" encoding="utf-8"?>\n{head}\n<vector xmlns:android="http://schemas.android.com/apk/res/android"\n{body}\n</vector>\n')
    write_web(shapes)


def web_part(p):
    """A path of the sprite: a line 1.5 wide with round ends unless it says otherwise (its symbol's defaults)."""
    a = [f'd="{p["d"]}"']
    if p["fill"]:
        a.append('fill="currentColor"')
    if p["even"]:
        a.append('fill-rule="evenodd"')
    if not p["stroke"]:
        a.append('stroke="none"')
    if p["width"] != 1.5:
        a.append(f'stroke-width="{p["width"]:g}"')
    if p["stroke"] and not p["cap"]:
        a.append('stroke-linecap="butt"')
    if p["stroke"] and not p["join"]:
        a.append('stroke-linejoin="miter"')
    return "<path " + " ".join(a) + "/>"


def seal_parts(drawable: Path):
    """One of core's seal drawables (which the notification and the car show too) as sprite paths."""
    xml = drawable.read_text()
    out = []
    for path in re.findall(r"<path ([^>]*)/>", xml):
        attr = dict(re.findall(r'android:(\w+)="([^"]*)"', path))
        a = [f'd="{attr["pathData"]}"', 'fill="currentColor"' if "fillColor" in attr else 'fill="none"']
        if "fillAlpha" in attr:
            a.append(f'fill-opacity="{attr["fillAlpha"]}"')
        if attr.get("fillType") == "evenOdd":
            a.append('fill-rule="evenodd"')
        a.append(f'stroke="currentColor" stroke-width="{attr["strokeWidth"]}"' if "strokeColor" in attr else 'stroke="none"')
        out.append("<path " + " ".join(a) + "/>")
    turn = re.search(r'android:rotation="([-\d.]+)" android:pivotX="([\d.]+)" android:pivotY="([\d.]+)"', xml)
    body = "".join(out)
    return f'<g transform="rotate({turn[1]} {turn[2]} {turn[3]})">{body}</g>' if turn else body


def write_web(shapes):
    """web/index.html's sprite, between its glyph markers: each glyph a <symbol> named in kebab case
    ("PlaySolid" is #i-play-solid), the seal and its outline (#i-sealed, #i-seal) and the seasons' kanji
    (#i-kanji-spring ...), as Android draws them. Only those web/src names in quotes ("play-solid"), so
    the first paint carries no glyph the page never shows: run this again after naming a new one there."""
    page = ROOT / "web/index.html"
    named = set()
    for f in (ROOT / "web/src").rglob("*"):
        if f.suffix in (".svelte", ".ts") and "pkg" not in f.parts:
            named.update(re.findall(r'"([a-z][a-z0-9-]*)"', f.read_text()))
    kebab = lambda name: re.sub(r"(?<!^)(?=[A-Z])", "-", name).lower()
    line = ' fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"'
    symbols = {kebab(name): f'viewBox="0 0 24 24"{line}>{"".join(web_part(p) for p in ps)}' for name, ps in shapes.items()}
    for name, file in (("sealed", "seal.xml"), ("seal", "seal_outline.xml")):
        symbols[name] = f'viewBox="0 0 24 24">{seal_parts(CORE / file)}'
    kanji = dict(re.findall(r'private const val (SPRING|SUMMER|AUTUMN|WINTER) = "([^"]+)"', (KOTLIN.parent / "Kanji.kt").read_text()))
    for season in ("SPRING", "SUMMER", "AUTUMN", "WINTER"):
        symbols[f"kanji-{season.lower()}"] = f'viewBox="0 0 24 24"><path fill="currentColor" d="{kanji[season]}"/>'
    symbols = [f'<symbol id="i-{id}" {body}</symbol>' for id, body in symbols.items() if id in named]
    start, end = "<!-- glyphs -->", "<!-- /glyphs -->"
    head, rest = page.read_text().split(start)
    tail = rest.split(end)[1]
    sprite = "\n    ".join(symbols)
    page.write_text(
        f"{head}{start}\n  <!-- Generated by tools/glyphs.py from Iconoir (MIT, https://iconoir.com), and nori's seal and kanji "
        f"drawn from Shippori Mincho (OFL-1.1); do not edit by hand. -->\n"
        f'  <svg xmlns="http://www.w3.org/2000/svg" style="display:none">\n    {sprite}\n  </svg>\n  {end}{tail}'
    )


KOTLIN_HEAD = '''package dev.nori.music.app.ui

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.PathFillType
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.PathParser
import androidx.compose.ui.unit.dp

// Generated by tools/glyphs.py from Iconoir (MIT, https://iconoir.com); do not edit by hand.

/** One path of a glyph: a line 1.5 wide with round caps and joins unless it says otherwise. */
private class Part(
    val d: String, val fill: Boolean = false, val evenOdd: Boolean = false,
    val stroke: Boolean = true, val width: Float = 1.5f, val cap: Boolean = true, val join: Boolean = true,
)

private fun glyph(flips: Boolean, vararg d: String) = parts(flips, *d.map { Part(it) }.toTypedArray())

private fun parts(flips: Boolean, vararg parts: Part) = lazy {
    ImageVector.Builder("glyph", 24.dp, 24.dp, 24f, 24f, autoMirror = flips).apply {
        for (part in parts) {
            addPath(
                PathParser().parsePathString(part.d).toNodes(),
                pathFillType = if (part.evenOdd) PathFillType.EvenOdd else PathFillType.NonZero,
                fill = if (part.fill) SolidColor(Color.Black) else null,
                stroke = if (part.stroke) SolidColor(Color.Black) else null,
                strokeLineWidth = part.width,
                strokeLineCap = if (part.cap) StrokeCap.Round else StrokeCap.Butt,
                strokeLineJoin = if (part.join) StrokeJoin.Round else StrokeJoin.Miter,
            )
        }
    }.build()
}

/** nori's icons: Iconoir's line glyphs, and its solid ones for the transport. */
object Iconoir {
'''

if __name__ == "__main__":
    main(Path(sys.argv[1]))
