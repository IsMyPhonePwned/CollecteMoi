#!/usr/bin/env python3
"""Linux/Docker fallback for scripts/overlay-icon-text.swift (letterbox + optional text)."""
from __future__ import annotations

import sys
from pathlib import Path

try:
    from PIL import Image, ImageDraw, ImageFont
except ImportError as e:  # pragma: no cover
    sys.stderr.write(f"Pillow required for launcher icon edits: {e}\n")
    sys.exit(1)

# Debian/Ubuntu `python3-pil` is often older than Pillow 9.1 (no Image.Resampling).
_RESAMPLE_LANCZOS = getattr(getattr(Image, "Resampling", Image), "LANCZOS", Image.LANCZOS)
_RESAMPLE_BICUBIC = getattr(getattr(Image, "Resampling", Image), "BICUBIC", Image.BICUBIC)


def fail(msg: str, code: int = 1) -> None:
    sys.stderr.write(msg + "\n")
    sys.exit(code)


def text_size(draw: ImageDraw.ImageDraw, label: str, font: ImageFont.ImageFont) -> tuple[int, int]:
    if hasattr(draw, "textbbox"):
        bbox = draw.textbbox((0, 0), label, font=font)
        return bbox[2] - bbox[0], bbox[3] - bbox[1]
    # Pillow < 8
    return draw.textsize(label, font=font)


def draw_rounded_rect(
    draw: ImageDraw.ImageDraw,
    box: list[int],
    radius: int,
    fill: tuple[int, int, int, int],
) -> None:
    if hasattr(draw, "rounded_rectangle"):
        draw.rounded_rectangle(box, radius=radius, fill=fill)
        return
    # Pillow < 9.1: approximate with rectangles + pieslices.
    x0, y0, x1, y1 = box
    r = max(0, min(radius, (x1 - x0) // 2, (y1 - y0) // 2))
    draw.rectangle([x0 + r, y0, x1 - r, y1], fill=fill)
    draw.rectangle([x0, y0 + r, x1, y1 - r], fill=fill)
    draw.pieslice([x0, y0, x0 + 2 * r, y0 + 2 * r], 180, 270, fill=fill)
    draw.pieslice([x1 - 2 * r, y0, x1, y0 + 2 * r], 270, 360, fill=fill)
    draw.pieslice([x0, y1 - 2 * r, x0 + 2 * r, y1], 90, 180, fill=fill)
    draw.pieslice([x1 - 2 * r, y1 - 2 * r, x1, y1], 0, 90, fill=fill)


def parse_color(hex_color: str) -> tuple[int, int, int]:
    value = hex_color.lstrip("#")
    if len(value) != 6:
        fail("invalid color")
    try:
        rgb = int(value, 16)
    except ValueError:
        fail("invalid color")
    return ((rgb >> 16) & 0xFF, (rgb >> 8) & 0xFF, rgb & 0xFF)


def load_font(name: str, size: int) -> ImageFont.ImageFont:
    candidates = {
        "rounded": [
            "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
        ],
        "serif": [
            "/usr/share/fonts/truetype/dejavu/DejaVuSerif-Bold.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSerif-Bold.ttf",
        ],
        "monospaced": [
            "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationMono-Bold.ttf",
        ],
        "system": [
            "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
        ],
    }
    for path in candidates.get(name, candidates["system"]):
        if Path(path).is_file():
            return ImageFont.truetype(path, size=size)
    return ImageFont.load_default()


def main() -> None:
    if len(sys.argv) != 14:
        fail(
            "usage: overlay-icon-text.py <source> <text> <output> <font> <size> "
            "<x> <y> <color> <badge> <badge-color> <opacity> <scale> <rotation>",
            2,
        )
    (
        _,
        source_path,
        label,
        output_path,
        font_name,
        size_s,
        x_s,
        y_s,
        color_s,
        badge_s,
        badge_color_s,
        opacity_s,
        scale_s,
        rotation_s,
    ) = sys.argv

    try:
        size_percent = int(size_s)
        x_percent = int(x_s)
        y_percent = int(y_s)
        opacity = int(opacity_s)
        scale = int(scale_s)
        rotation = int(rotation_s)
    except ValueError:
        fail("invalid launcher text style")

    label = label.strip()
    if not (
        8 <= size_percent <= 32
        and 0 <= x_percent <= 100
        and 0 <= y_percent <= 100
        and 0 <= opacity <= 100
        and 40 <= scale <= 180
        and -180 <= rotation <= 180
        and font_name in {"system", "rounded", "serif", "monospaced"}
        and badge_s in {"true", "false"}
        and len(label) <= 24
        and all(ord(c) >= 32 for c in label)
    ):
        fail("invalid launcher text style")

    text_color = parse_color(color_s)
    badge_color = parse_color(badge_color_s)

    try:
        source = Image.open(source_path).convert("RGBA")
    except OSError as e:
        fail(f"could not read source icon: {e}")

    side = 1024
    canvas = Image.new("RGBA", (side, side), (0, 0, 0, 0))
    src_w, src_h = source.size
    max_dim = max(src_w, src_h) or 1
    image_side = side * scale / 100.0
    draw_w = max(1, int(round(image_side * src_w / max_dim)))
    draw_h = max(1, int(round(image_side * src_h / max_dim)))
    resized = source.resize((draw_w, draw_h), _RESAMPLE_LANCZOS)

    if rotation % 360 != 0:
        resized = resized.rotate(-rotation, expand=True, resample=_RESAMPLE_BICUBIC)
        draw_w, draw_h = resized.size

    ox = (side - draw_w) // 2
    oy = (side - draw_h) // 2
    canvas.alpha_composite(resized, (ox, oy))

    if label:
        font_size = max(8, int(side * size_percent / 100))
        font = load_font(font_name, font_size)
        draw = ImageDraw.Draw(canvas)
        text_w, text_h = text_size(draw, label, font)
        if text_w > side - 32 and text_w > 0:
            fitted = max(8, int(font_size * (side - 32) / text_w))
            font = load_font(font_name, fitted)
            text_w, text_h = text_size(draw, label, font)

        center_x = side * x_percent / 100.0
        center_y = side * y_percent / 100.0
        text_x = int(center_x - text_w / 2)
        text_y = int(center_y - text_h / 2)

        if badge_s == "true" and opacity > 0:
            pad_x = max(22, int(font_size * 0.16))
            pad_y = max(14, int(font_size * 0.12))
            radius = max(4, int(font_size * 0.18))
            badge = [
                text_x - pad_x,
                text_y - pad_y,
                text_x + text_w + pad_x,
                text_y + text_h + pad_y,
            ]
            fill = (*badge_color, int(255 * opacity / 100))
            draw_rounded_rect(draw, badge, radius, fill)

        draw.text((text_x, text_y), label, font=font, fill=(*text_color, 255))

    out = Path(output_path)
    out.parent.mkdir(parents=True, exist_ok=True)
    canvas.save(out, format="PNG")


if __name__ == "__main__":
    main()
