"""Generate original, minimal test geometry. Requires fonttools, only to regenerate.
The resulting font is a test fixture, not a redistributed system font.
"""
from pathlib import Path
from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.feaLib.builder import addOpenTypeFeaturesFromString

builder = FontBuilder(1000, isTTF=True)
order = [".notdef", "space", "O", "f", "i", "fi", "e", "eacute"]
builder.setupGlyphOrder(order)
builder.setupCharacterMap({32: "space", 79: "O", 102: "f", 105: "i", 101: "e", 233: "eacute", 0x53E3: "O"})
glyphs = {}
for name in order:
    pen = TTGlyphPen(None)
    if name != "space":
        pen.moveTo((100, 0))
        for p in [(900, 0), (900, 1000), (100, 1000)]:
            pen.lineTo(p)
        pen.closePath()
        pen.moveTo((300, 200))
        for p in [(300, 800), (700, 800), (700, 200)]:
            pen.lineTo(p)
        pen.closePath()
    glyphs[name] = pen.glyph()
builder.setupGlyf(glyphs)
builder.setupHorizontalMetrics({name: (300 if name == "space" else 1000, 0 if name == "space" else 100) for name in order})
builder.setupHorizontalHeader(ascent=1000, descent=0)
builder.setupNameTable({"familyName": "Lagui Test", "styleName": "Regular", "uniqueFontIdentifier": "Lagui-Test-1", "fullName": "Lagui Test", "psName": "Lagui-Test"})
builder.setupOS2(sTypoAscender=1000, sTypoDescender=0, usWinAscent=1000, usWinDescent=0)
builder.setupPost()
builder.setupMaxp()
addOpenTypeFeaturesFromString(builder.font, "feature liga { sub f i by fi; } liga;")
builder.font["head"].created = builder.font["head"].modified = 2082844800
builder.font.recalcTimestamp = False
target = Path(__file__).resolve().parents[1] / "tests/assets/test-outline.ttf"
target.parent.mkdir(parents=True, exist_ok=True)
builder.save(target)
