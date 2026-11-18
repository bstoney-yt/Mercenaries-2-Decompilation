"""Side-by-side contact sheet and numeric report for one comparison case.

    uv run --no-project --with pillow --with numpy python -I sheet.py <case dir> [--title T]

Reads <case>/xenia/*.png and <case>/m2/*.ppm|png (sorted by name, paired in
order; the shorter list sets the rows), writes <case>/sheet.png and
<case>/metrics.md. Images stay local. Metrics per frame: mean luma, mean RGB,
saturation (mean HSV S), contrast (luma std), edge density (share of pixels
whose luma gradient exceeds 24), and the top-band/bottom-band luma (sky vs
ground) so lighting and composition differences show up as numbers.
"""

from __future__ import annotations

import argparse
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFont

W = 480  # each panel's width in the sheet


def load(p: Path) -> Image.Image:
    return Image.open(p).convert("RGB")


def metrics(im: Image.Image) -> dict:
    a = np.asarray(im.resize((320, 180)), dtype=np.float32)
    luma = a @ np.array([0.299, 0.587, 0.114], dtype=np.float32)
    hsv = np.asarray(im.resize((320, 180)).convert("HSV"), dtype=np.float32)
    gy, gx = np.gradient(luma)
    h = luma.shape[0]
    return {
        "luma": float(luma.mean()),
        "rgb": tuple(int(v) for v in a.reshape(-1, 3).mean(0)),
        "sat": float(hsv[..., 1].mean()),
        "contrast": float(luma.std()),
        "edges": float((np.hypot(gx, gy) > 24).mean()),
        "top": float(luma[: h // 3].mean()),
        "bottom": float(luma[-h // 3 :].mean()),
    }


def fmt(m: dict) -> str:
    return (f"luma {m['luma']:.0f} rgb {m['rgb']} sat {m['sat']:.0f} con {m['contrast']:.0f} "
            f"edges {m['edges']:.3f} top/bot {m['top']:.0f}/{m['bottom']:.0f}")


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("case")
    ap.add_argument("--title", default=None)
    a = ap.parse_args()
    case = Path(a.case)
    xs = sorted((case / "xenia").glob("*.png"))
    ms = sorted([*(case / "m2").glob("*.ppm"), *(case / "m2").glob("*.png")])
    n = min(len(xs), len(ms))
    if n == 0:
        raise SystemExit(f"nothing to pair: {len(xs)} xenia, {len(ms)} m2 frames")
    font = ImageFont.truetype("arial.ttf", 14)
    big = ImageFont.truetype("arial.ttf", 20)
    rows, lines = [], []
    for x, m in zip(xs[:n], ms[:n]):
        ix, im = load(x), load(m)
        ph = round(W * 9 / 16)
        px = ix.resize((W, round(W * ix.height / ix.width))).crop((0, 0, W, ph))
        pm = im.resize((W, round(W * im.height / im.width))).crop((0, 0, W, ph))
        mx, mm = metrics(ix), metrics(im)
        row = Image.new("RGB", (2 * W + 30, ph + 40), (24, 24, 24))
        row.paste(px, (10, 30))
        row.paste(pm, (W + 20, 30))
        d = ImageDraw.Draw(row)
        d.text((10, 6), f"360/Xenia  {x.stem}", fill=(255, 220, 120), font=font)
        d.text((W + 20, 6), f"m2-alpha  {m.stem}", fill=(140, 210, 255), font=font)
        rows.append(row)
        lines.append(f"| {x.stem} | {fmt(mx)} | {m.stem} | {fmt(mm)} | {mm['luma'] - mx['luma']:+.0f} |")
    title = a.title or case.name
    sheet = Image.new("RGB", (rows[0].width, 40 + sum(r.height for r in rows)), (16, 16, 16))
    ImageDraw.Draw(sheet).text((10, 8), f"{title}: Xenia (left) vs m2-alpha (right)", fill="white", font=big)
    y = 40
    for r in rows:
        sheet.paste(r, (0, y))
        y += r.height
    sheet.save(case / "sheet.png")
    md = [f"# {title}: frame metrics", "",
          "| Xenia frame | Xenia | m2 frame | m2-alpha | luma diff |", "|---|---|---|---|---|", *lines, ""]
    (case / "metrics.md").write_text("\n".join(md), encoding="utf-8")
    print(f"{case / 'sheet.png'}: {n} pairs")
    print("\n".join(lines))


if __name__ == "__main__":
    main()
