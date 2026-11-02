#!/usr/bin/env python3
"""Export the approved Honk identity from one SVG geometry; never touches installed apps."""
import hashlib
import json
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[1]
BRAND = ROOT / "assets/brand"
PUBLIC = ROOT / "web/console/public/brand"
ICONS = ROOT / "src-tauri/icons"
SVG = "http://www.w3.org/2000/svg"
ET.register_namespace("", SVG)


def variant(primary, secondary):
    root = ET.parse(BRAND / "honk-mark.svg").getroot()
    for element in root.iter():
        if element.get("fill") == "#2563eb":
            element.set("fill", primary)
        elif element.get("fill") == "#38bdf8":
            element.set("fill", secondary)
    return root


def save_svg(path, root):
    path.parent.mkdir(parents=True, exist_ok=True)
    ET.indent(root, space="  ")
    ET.ElementTree(root).write(path, encoding="unicode", xml_declaration=False)
    with path.open("a") as file:
        file.write("\n")


def render(source, target, size):
    target.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(["rsvg-convert", "--width", str(size), "--height", str(size),
                    "--output", str(target), str(source)], check=True)


def canonical_icns(path):
    # Tauri's ICNS writer iterates a hash map. Sort records without changing pixels,
    # keeping legacy RGB/alpha pairs adjacent for older icon readers.
    data = path.read_bytes()
    if data[:4] != b"icns" or struct.unpack(">I", data[4:8])[0] != len(data):
        raise ValueError("invalid generated ICNS header")
    records = []
    offset = 8
    while offset < len(data):
        length = struct.unpack(">I", data[offset + 4:offset + 8])[0]
        if length < 8 or offset + length > len(data):
            raise ValueError("invalid generated ICNS record")
        records.append(data[offset:offset + length])
        offset += length
    pairs = {b"s8mk": b"is32", b"l8mk": b"il32", b"h8mk": b"ih32", b"t8mk": b"it32"}
    records.sort(key=lambda record: (pairs.get(record[:4], record[:4]), record[:4] in pairs))
    path.write_bytes(data[:8] + b"".join(records))


def main():
    if not shutil.which("rsvg-convert"):
        raise SystemExit("rsvg-convert is required (librsvg); install it before exporting")
    PUBLIC.mkdir(parents=True, exist_ok=True)
    ICONS.mkdir(parents=True, exist_ok=True)
    generated = []
    for name, primary, secondary in (
        ("honk-mark-dark", "#ffffff", "#38bdf8"),
        ("honk-mark-mono", "#0b1220", "#0b1220"),
        ("honk-mark-white", "#ffffff", "#ffffff"),
    ):
        path = BRAND / (name + ".svg")
        save_svg(path, variant(primary, secondary))
        generated.append(path)

    # The app tile has its own canvas and padding; all mark paths still come from the master.
    mark = variant("#ffffff", "#38bdf8")
    tile = ET.Element("{%s}svg" % SVG, {"width": "1024", "height": "1024", "viewBox": "0 0 1024 1024"})
    ET.SubElement(tile, "{%s}title" % SVG).text = "Honk Proxy"
    definitions = mark.find("{%s}defs" % SVG)
    tile.append(definitions)
    gradient = ET.SubElement(definitions, "{%s}linearGradient" % SVG,
                             {"id": "tile", "x1": "0", "y1": "0", "x2": "0.8", "y2": "1"})
    ET.SubElement(gradient, "{%s}stop" % SVG, {"stop-color": "#3378ff", "offset": "0"})
    ET.SubElement(gradient, "{%s}stop" % SVG, {"stop-color": "#0950db", "offset": "1"})
    ET.SubElement(tile, "{%s}rect" % SVG, {"x": "32", "y": "32", "width": "960", "height": "960", "rx": "218", "fill": "url(#tile)"})
    group = ET.SubElement(tile, "{%s}g" % SVG, {"transform": "translate(224 224) scale(1.8)"})
    for child in list(mark):
        if child.tag == "{%s}use" % SVG:
            group.append(child)
    app = BRAND / "app-icon.svg"
    save_svg(app, tile)
    generated.append(app)

    # A single adaptive SVG favicon covers modern browsers; ICO/PNG cover older consumers.
    favicon = variant("#2563eb", "#38bdf8")
    for child in favicon:
        if child.tag == "{%s}use" % SVG and child.get("fill") == "#2563eb":
            child.set("class", "honk-primary")
    ET.SubElement(favicon, "{%s}style" % SVG).text = "@media (prefers-color-scheme: dark) { .honk-primary { fill: #ffffff; } }"
    save_svg(PUBLIC / "favicon.svg", favicon)
    generated.append(PUBLIC / "favicon.svg")

    for name in ("honk-mark", "honk-mark-dark", "honk-mark-mono", "honk-mark-white"):
        for size in (16, 24, 32, 48, 64, 128, 256, 512, 1024):
            target = BRAND / "png" / (name + "-" + str(size) + ".png")
            render(BRAND / (name + ".svg"), target, size)
            generated.append(target)

    # Tauri's installed CLI exports native ICO, ICNS and Windows store sizes from the same tile.
    with tempfile.TemporaryDirectory(prefix="hps-brand-icons-") as temporary:
        subprocess.run(["pnpm", "exec", "tauri", "icon", str(app), "--output", temporary], cwd=ROOT, check=True)
        for source in sorted(Path(temporary).iterdir()):
            if source.is_file() and source.suffix in (".png", ".ico", ".icns"):
                target = ICONS / source.name
                shutil.copyfile(source, target)
                if target.suffix == ".icns":
                    canonical_icns(target)
                generated.append(target)
    for size in (16, 32, 48):
        target = PUBLIC / ("favicon-" + str(size) + ".png")
        render(BRAND / "honk-mark.svg", target, size)
        generated.append(target)
    for name, size in (("apple-touch-icon.png", 180), ("icon-192.png", 192), ("icon-512.png", 512)):
        target = PUBLIC / name
        render(app, target, size)
        generated.append(target)
    shutil.copyfile(ICONS / "icon.ico", PUBLIC / "favicon.ico")
    generated.append(PUBLIC / "favicon.ico")
    for size, suffix in ((22, ""), (44, "@2x")):
        target = ICONS / ("tray-template" + suffix + ".png")
        render(BRAND / "honk-mark-mono.svg", target, size)
        generated.append(target)
    render(app, BRAND / "app-icon-1024.png", 1024)
    generated.append(BRAND / "app-icon-1024.png")

    manifest = {"master": "assets/brand/honk-mark.svg",
                "master_sha256": hashlib.sha256((BRAND / "honk-mark.svg").read_bytes()).hexdigest(),
                "files": {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest()
                          for p in sorted(generated)}}
    (BRAND / "assets-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print("Exported %s brand assets from %s" % (len(generated), manifest["master"]))


if __name__ == "__main__":
    main()
