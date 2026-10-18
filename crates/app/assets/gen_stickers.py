#!/usr/bin/env python3
"""Generate the built-in sticker library.

Follows the same shape as gen_luts.py: everything in
crates/app/assets/stickers/ is produced here and committed, so a build has
its stickers with no download step and no licence attached.

Art is drawn at 4x and downsampled, because Pillow has no anti-aliasing
for polygons and the sticker set is mostly polygons. Output is RGBA with a
transparent background -- the render path composites these with ffmpeg's
`overlay`, which is what preserves the transparency.

Run from the repo root:
    python crates/app/assets/gen_stickers.py
"""

import json
import math
import os

from PIL import Image, ImageDraw, ImageFilter

SS = 4                 # supersample factor
SIZE = 256             # final canvas, square
OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), 'stickers')

CATALOG = []           # (id, name, category) -- kept in step with the files


def canvas():
    img = Image.new('RGBA', (SIZE * SS, SIZE * SS), (0, 0, 0, 0))
    return img, ImageDraw.Draw(img)


def finish(img, sid, name, category):
    """Downsample and write, then record the entry."""
    out = img.resize((SIZE, SIZE), Image.LANCZOS)
    out.save(os.path.join(OUT, sid + '.png'), optimize=True)
    CATALOG.append((sid, name, category))
    return out


def render(fn, sid, name, category):
    return finish(fn(), sid, name, category)


def scale(pts):
    return [(x * SS, y * SS) for x, y in pts]


def star_points(cx, cy, r_out, r_in, n, rot=-math.pi / 2):
    pts = []
    for i in range(n * 2):
        r = r_out if i % 2 == 0 else r_in
        a = rot + i * math.pi / n
        pts.append((cx + r * math.cos(a), cy + r * math.sin(a)))
    return pts


def regular(cx, cy, r, n, rot=0.0):
    return [(cx + r * math.cos(rot + i * 2 * math.pi / n),
             cy + r * math.sin(rot + i * 2 * math.pi / n)) for i in range(n)]


def shadow_of(img, offset=(0, 6), blur=6, alpha=90):
    """Drop a soft shadow under whatever alpha is already in `img`."""
    a = img.split()[3]
    sh = Image.new('RGBA', img.size, (0, 0, 0, 0))
    sh.putalpha(a.filter(ImageFilter.GaussianBlur(blur * SS)).point(
        lambda v: int(v * alpha / 255)))
    shifted = Image.new('RGBA', img.size, (0, 0, 0, 0))
    shifted.paste(sh, (offset[0] * SS, offset[1] * SS))
    return shifted


def with_shadow(img):
    sh = shadow_of(img)
    out = img.copy()
    out.alpha_composite(sh)
    return out


def glow(img, radius, strength=140):
    a = img.split()[3]
    g = Image.new('RGBA', img.size, (255, 255, 255, 0))
    g.putalpha(a.filter(ImageFilter.GaussianBlur(radius * SS)).point(
        lambda v: min(255, int(v * strength / 255))))
    out = img.copy()
    out.alpha_composite(g)
    return out


def ngon_path(cx, cy, r, n, rot):
    """Regular polygon, final-size units in, supersampled points out."""
    return scale(regular(cx, cy, r, n, rot))


def rounded_poly(d, pts, r, fill, outline=None, width=0):
    d.polygon(pts, fill=fill)
    if outline:
        d.line(pts + [pts[0]], fill=outline, width=width, joint='curve')


# --------------------------------------------------------------------------
# 1. Shapes
# --------------------------------------------------------------------------
def s_circle():
    img, d = canvas()
    d.ellipse([40 * SS, 40 * SS, 216 * SS, 216 * SS], fill=(239, 68, 68, 255))
    return with_shadow(img)


def s_ring():
    img, d = canvas()
    d.ellipse([36 * SS, 36 * SS, 220 * SS, 220 * SS], outline=(250, 204, 21, 255),
              width=18 * SS)
    return with_shadow(img)


def s_rounded_square():
    img, d = canvas()
    d.rounded_rectangle([44 * SS, 44 * SS, 212 * SS, 212 * SS], radius=34 * SS,
                        fill=(59, 130, 246, 255))
    return with_shadow(img)


def s_triangle():
    img, d = canvas()
    d.polygon(scale([(128, 42), (216, 208), (40, 208)]), fill=(34, 197, 94, 255))
    return with_shadow(img)


def s_hexagon():
    img, d = canvas()
    d.polygon(ngon_path(128, 128, 90, 6, math.pi / 6),
              fill=(168, 85, 247, 255))
    return with_shadow(img)


def s_pentagon():
    img, d = canvas()
    d.polygon(ngon_path(128, 128, 90, 5, -math.pi / 2),
              fill=(244, 63, 94, 255))
    return with_shadow(img)


def s_diamond():
    img, d = canvas()
    d.polygon(scale([(128, 34), (222, 128), (128, 222), (34, 128)]),
              fill=(14, 165, 233, 255))
    return with_shadow(img)


def s_star():
    img, d = canvas()
    d.polygon(scale(star_points(128, 128, 96, 40, 5)),
              fill=(250, 204, 21, 255))
    return with_shadow(img)


def s_hex_star():
    img, d = canvas()
    d.polygon(scale(star_points(128, 128, 96, 44, 6)),
              fill=(251, 146, 60, 255))
    return with_shadow(img)


def s_burst():
    img, d = canvas()
    d.polygon(scale(star_points(128, 128, 100, 58, 12)),
              fill=(250, 204, 21, 255))
    return with_shadow(img)


def s_blob():
    img, d = canvas()
    pts = []
    for i in range(10):
        a = i * 2 * math.pi / 10
        r = 84 + 16 * math.sin(i * 2.3)
        pts.append((128 + r * math.cos(a), 128 + r * math.sin(a)))
    d.polygon(scale(pts), fill=(45, 212, 191, 255))
    return with_shadow(img)


def s_chevron():
    img, d = canvas()
    w = 22 * SS
    d.line(scale([(70, 60), (128, 128), (70, 196)]), fill=(234, 88, 12, 255),
           width=w, joint='curve')
    d.line(scale([(140, 60), (198, 128), (140, 196)]), fill=(234, 88, 12, 255),
           width=w, joint='curve')
    return img


def s_cross():
    img, d = canvas()
    w = 26 * SS
    d.line(scale([(78, 78), (178, 178)]), fill=(220, 38, 38, 255), width=w)
    d.line(scale([(178, 78), (78, 178)]), fill=(220, 38, 38, 255), width=w)
    return with_shadow(img)


def s_capsule():
    img, d = canvas()
    d.rounded_rectangle([36 * SS, 96 * SS, 220 * SS, 160 * SS], radius=32 * SS,
                        fill=(236, 72, 153, 255))
    return with_shadow(img)


def s_teardrop():
    img, d = canvas()
    d.ellipse([68 * SS, 108 * SS, 188 * SS, 214 * SS], fill=(56, 189, 248, 255))
    d.polygon(scale([(128, 40), (188, 130), (68, 130)]), fill=(56, 189, 248, 255))
    return with_shadow(img)


def s_moon():
    img, d = canvas()
    d.ellipse([50 * SS, 50 * SS, 206 * SS, 206 * SS], fill=(253, 224, 71, 255))
    d.ellipse([96 * SS, 34 * SS, 224 * SS, 178 * SS], fill=(0, 0, 0, 0))
    return with_shadow(img)


def s_sun():
    img, d = canvas()
    for i in range(12):
        a = i * math.pi / 6
        d.line(scale([(128 + 58 * math.cos(a), 128 + 58 * math.sin(a)),
                      (128 + 92 * math.cos(a), 128 + 92 * math.sin(a))]),
               fill=(251, 191, 36, 255), width=10 * SS)
    d.ellipse([84 * SS, 84 * SS, 172 * SS, 172 * SS], fill=(253, 224, 71, 255))
    return glow(img, 5)


def s_squircle():
    img, d = canvas()
    d.rounded_rectangle([48 * SS, 48 * SS, 208 * SS, 208 * SS], radius=72 * SS,
                        fill=(139, 92, 246, 255))
    return with_shadow(img)


def s_trapezoid():
    img, d = canvas()
    d.polygon(scale([(58, 76), (198, 76), (228, 196), (28, 196)]),
              fill=(20, 184, 166, 255))
    return with_shadow(img)


SHAPES = [
    ('circle', 'Circle', s_circle),
    ('ring', 'Ring', s_ring),
    ('rounded-square', 'Rounded Square', s_rounded_square),
    ('triangle', 'Triangle', s_triangle),
    ('hexagon', 'Hexagon', s_hexagon),
    ('pentagon', 'Pentagon', s_pentagon),
    ('diamond', 'Diamond', s_diamond),
    ('star', 'Star', s_star),
    ('hex-star', 'Six Point Star', s_hex_star),
    ('burst', 'Burst', s_burst),
    ('blob', 'Blob', s_blob),
    ('chevron', 'Chevron', s_chevron),
    ('cross', 'Cross', s_cross),
    ('capsule', 'Capsule', s_capsule),
    ('teardrop', 'Teardrop', s_teardrop),
    ('moon', 'Moon', s_moon),
    ('sun', 'Sun', s_sun),
    ('squircle', 'Squircle', s_squircle),
    ('trapezoid', 'Trapezoid', s_trapezoid),
]


# --------------------------------------------------------------------------
# 2. Hearts
# --------------------------------------------------------------------------
def heart_path(cx, cy, w, h):
    """Classic two-lobe heart as a polygon, sampled finely enough to
    round the lobes."""
    pts = []
    for i in range(161):
        t = 2 * math.pi * i / 160
        x = 16 * math.sin(t) ** 3
        y = 13 * math.cos(t) - 5 * math.cos(2 * t) - 2 * math.cos(3 * t) - math.cos(4 * t)
        pts.append((cx + x * w / 34.0, cy - y * h / 34.0))
    return pts


def h_classic():
    img, d = canvas()
    d.polygon(scale(heart_path(128, 138, 168, 160)), fill=(239, 68, 68, 255))
    return with_shadow(img)


def h_outline():
    img, d = canvas()
    d.polygon(scale(heart_path(128, 138, 168, 160)), outline=(244, 63, 94, 255),
              width=12 * SS)
    return img


def h_double():
    img, d = canvas()
    d.polygon(scale(heart_path(96, 150, 132, 126)), fill=(244, 63, 94, 255))
    d.polygon(scale(heart_path(160, 150, 132, 126)), fill=(251, 113, 133, 255))
    return with_shadow(img)


def h_broken():
    img, d = canvas()
    d.polygon(scale(heart_path(128, 138, 168, 160)), fill=(239, 68, 68, 255))
    d.polygon(scale([(124, 34), (140, 96), (112, 150), (140, 216), (124, 244)]),
              fill=(0, 0, 0, 0))
    return img


def h_burst():
    img, d = canvas()
    d.polygon(scale(star_points(128, 128, 112, 74, 14)), fill=(244, 63, 94, 210))
    d.polygon(scale(heart_path(128, 134, 118, 112)), fill=(255, 255, 255, 255))
    return glow(img, 4)


def h_frame():
    img, d = canvas()
    d.polygon(scale(heart_path(128, 138, 168, 160)), outline=(236, 72, 153, 255),
              width=11 * SS)
    d.polygon(scale(heart_path(128, 138, 112, 106)), outline=(251, 207, 232, 255),
              width=5 * SS)
    return img


def h_sparkle():
    img, d = canvas()
    d.polygon(scale(heart_path(128, 142, 150, 144)), fill=(244, 63, 94, 255))
    d.polygon(scale(star_points(196, 66, 26, 6, 4)), fill=(253, 224, 71, 255))
    return glow(img, 5)


def h_pulse():
    img, d = canvas()
    d.polygon(scale(heart_path(128, 138, 156, 148)), fill=(236, 72, 153, 255))
    pts = [(24, 128), (68, 128), (84, 92), (104, 168), (124, 116),
           (140, 128), (184, 128)]
    d.line(scale(pts), fill=(255, 255, 255, 255), width=7 * SS, joint='curve')
    return img


def h_globe():
    img, d = canvas()
    d.polygon(scale(heart_path(128, 138, 168, 160)), fill=(239, 68, 68, 255))
    d.ellipse([84 * SS, 76 * SS, 172 * SS, 156 * SS], outline=(255, 255, 255, 230),
              width=6 * SS)
    d.line(scale([(84, 116), (172, 116)]), fill=(255, 255, 255, 230), width=5 * SS)
    d.line(scale([(128, 76), (128, 156)]), fill=(255, 255, 255, 230), width=5 * SS)
    return with_shadow(img)


def h_arrow():
    img, d = canvas()
    d.polygon(scale(heart_path(112, 150, 132, 126)), fill=(244, 63, 94, 255))
    d.line(scale([(150, 92), (206, 128), (150, 164)]), fill=(255, 255, 255, 255),
           width=14 * SS, joint='curve')
    return img


HEARTS = [
    ('heart', 'Heart', h_classic),
    ('heart-outline', 'Heart Outline', h_outline),
    ('heart-double', 'Two Hearts', h_double),
    ('heart-broken', 'Broken Heart', h_broken),
    ('heart-burst', 'Heart Burst', h_burst),
    ('heart-frame', 'Heart Frame', h_frame),
    ('heart-sparkle', 'Heart Sparkle', h_sparkle),
    ('heart-pulse', 'Heartbeat', h_pulse),
    ('heart-globe', 'Heart Globe', h_globe),
    ('heart-arrow', 'Heart Arrow', h_arrow),
]


# --------------------------------------------------------------------------
# 3. Arrows and lightning
# --------------------------------------------------------------------------
def arrow_shape(d, cx, cy, length, thick, head, angle, fill):
    """Arrow in final-size units. Supersampling happens once, here."""
    pts = []
    hl, hw = length / 2.0, thick / 2.0
    for x, y in [(-hl, -hw), (hl - head, -hw), (hl - head, -head * 1.15),
                 (hl, 0), (hl - head, head * 1.15), (hl - head, hw), (-hl, hw)]:
        pts.append((cx + x * math.cos(angle) - y * math.sin(angle),
                    cy + x * math.sin(angle) + y * math.cos(angle)))
    d.polygon(scale(pts), fill=fill)


def a_right():
    img, d = canvas()
    arrow_shape(d, 128, 128, 150, 44, 54, 0,
                (37, 99, 235, 255))
    return with_shadow(img)


def a_left():
    img, d = canvas()
    arrow_shape(d, 128, 128, 150, 44, 54, math.pi,
                (37, 99, 235, 255))
    return with_shadow(img)


def a_up():
    img, d = canvas()
    arrow_shape(d, 128, 128, 150, 44, 54, -math.pi / 2,
                (37, 99, 235, 255))
    return with_shadow(img)


def a_down():
    img, d = canvas()
    arrow_shape(d, 128, 128, 150, 44, 54, math.pi / 2,
                (37, 99, 235, 255))
    return with_shadow(img)


def a_double():
    img, d = canvas()
    arrow_shape(d, 92, 128, 110, 34, 40, 0,
                (5, 150, 105, 255))
    arrow_shape(d, 164, 128, 110, 34, 40, math.pi,
                (16, 185, 129, 255))
    return img


def a_curved():
    img, d = canvas()
    pts = [(54, 178), (54, 110), (128, 84), (196, 110)]
    d.line(scale(pts), fill=(124, 58, 237, 255), width=22 * SS, joint='curve')
    d.polygon(scale([(214, 92), (232, 122), (192, 122)]), fill=(124, 58, 237, 255))
    return img


def a_block():
    img, d = canvas()
    d.rounded_rectangle([44 * SS, 100 * SS, 168 * SS, 156 * SS], radius=10 * SS,
                        fill=(220, 38, 38, 255))
    d.polygon(scale([(158, 72), (222, 128), (158, 184)]), fill=(220, 38, 38, 255))
    return with_shadow(img)


def a_circle():
    img, d = canvas()
    d.ellipse([48 * SS, 48 * SS, 208 * SS, 208 * SS], fill=(217, 70, 239, 255))
    arrow_shape(d, 128, 128, 96, 26, 32, 0,
                (255, 255, 255, 255))
    return with_shadow(img)


def a_bolt():
    img, d = canvas()
    d.polygon(scale([(146, 26), (78, 140), (124, 140), (106, 230), (186, 108),
                     (138, 108)]), fill=(250, 204, 21, 255))
    return glow(img, 6)


def a_tail():
    img, d = canvas()
    d.line(scale([(44, 92), (150, 92)]), fill=(244, 63, 94, 255), width=18 * SS)
    d.line(scale([(44, 128), (150, 128)]), fill=(244, 63, 94, 255), width=18 * SS)
    d.line(scale([(44, 164), (150, 164)]), fill=(244, 63, 94, 255), width=18 * SS)
    arrow_shape(d, 186, 128, 76, 34, 30, 0,
                (244, 63, 94, 255))
    return with_shadow(img)


def a_zigzag():
    img, d = canvas()
    d.line(scale([(40, 176), (86, 96), (132, 168), (178, 88), (216, 152)]),
           fill=(250, 204, 21, 255), width=18 * SS, joint='curve')
    return img


def a_pin():
    img, d = canvas()
    d.polygon(scale(heart_path(128, 116, 120, 114)), fill=(225, 29, 72, 255))
    d.polygon(scale([(120, 168), (136, 168), (128, 230)]), fill=(225, 29, 72, 255))
    return with_shadow(img)


def a_pin_round():
    img, d = canvas()
    d.ellipse([54 * SS, 34 * SS, 202 * SS, 182 * SS], fill=(225, 29, 72, 255))
    d.ellipse([86 * SS, 62 * SS, 128 * SS, 104 * SS], fill=(255, 255, 255, 255))
    d.polygon(scale([(112, 174), (144, 174), (128, 232)]), fill=(225, 29, 72, 255))
    return with_shadow(img)


def a_retro():
    img, d = canvas()
    d.polygon(scale([(64, 78), (192, 78), (224, 128), (192, 178), (64, 178),
                     (32, 128)]), fill=(244, 114, 182, 255))
    d.polygon(scale([(96, 108), (160, 108), (182, 128), (160, 148), (96, 148),
                     (74, 128)]), fill=(255, 255, 255, 240))
    return with_shadow(img)


ARROWS = [
    ('arrow-right', 'Arrow Right', a_right),
    ('arrow-left', 'Arrow Left', a_left),
    ('arrow-up', 'Arrow Up', a_up),
    ('arrow-down', 'Arrow Down', a_down),
    ('arrow-double', 'Double Arrow', a_double),
    ('arrow-curved', 'Curved Arrow', a_curved),
    ('arrow-block', 'Block Arrow', a_block),
    ('arrow-circle', 'Circle Arrow', a_circle),
    ('bolt', 'Lightning Bolt', a_bolt),
    ('arrow-tail', 'Motion Arrow', a_tail),
    ('arrow-zigzag', 'Zigzag', a_zigzag),
    ('pin-heart', 'Heart Pin', a_pin),
    ('pin-round', 'Round Pin', a_pin_round),
    ('arrow-retro', 'Retro Arrow', a_retro),
]


# --------------------------------------------------------------------------
# 4. Sparkles
# --------------------------------------------------------------------------
def s_sparkle4():
    img, d = canvas()
    d.polygon(scale(star_points(128, 128, 104, 26, 4)), fill=(250, 204, 21, 255))
    return glow(img, 7)


def s_sparkle_small():
    img, d = canvas()
    d.polygon(scale(star_points(128, 128, 72, 18, 4)), fill=(253, 224, 71, 255))
    return glow(img, 5)


def s_star5():
    img, d = canvas()
    d.polygon(scale(star_points(128, 128, 98, 42, 5)), fill=(250, 204, 21, 255))
    return glow(img, 5)


def s_star8():
    img, d = canvas()
    d.polygon(scale(star_points(128, 128, 96, 44, 8)), fill=(253, 224, 71, 255))
    return glow(img, 5)


def s_star_outline():
    img, d = canvas()
    d.polygon(scale(star_points(128, 128, 98, 42, 5)), outline=(250, 204, 21, 255),
              width=10 * SS)
    return img


def s_shooting():
    img, d = canvas()
    d.polygon(scale([(56, 200), (150, 96), (172, 118), (86, 218)]),
              fill=(253, 224, 71, 200))
    d.polygon(scale([(112, 128), (168, 56), (186, 76), (136, 142)]),
              fill=(253, 224, 71, 120))
    d.polygon(scale(star_points(186, 60, 44, 10, 4)), fill=(255, 255, 255, 255))
    return glow(img, 6)


def s_trail():
    img, d = canvas()
    d.polygon(scale(star_points(180, 76, 52, 12, 4)), fill=(253, 224, 71, 255))
    d.polygon(scale(star_points(120, 130, 30, 7, 4)), fill=(253, 224, 71, 190))
    d.polygon(scale(star_points(76, 176, 20, 5, 4)), fill=(253, 224, 71, 130))
    return glow(img, 6)


def s_glow_dot():
    img, d = canvas()
    d.ellipse([80 * SS, 80 * SS, 176 * SS, 176 * SS], fill=(255, 255, 255, 255))
    return glow(img, 16, 190)


def s_twinkle():
    img, d = canvas()
    d.polygon(scale(star_points(128, 128, 100, 14, 4)), fill=(255, 255, 255, 255))
    d.polygon(scale(star_points(128, 128, 66, 9, 4)), fill=(255, 255, 255, 255))
    return glow(img, 10)


def s_star_stack():
    img, d = canvas()
    d.polygon(scale(star_points(96, 96, 62, 26, 5)), fill=(251, 191, 36, 255))
    d.polygon(scale(star_points(162, 152, 56, 24, 5)), fill=(250, 204, 21, 255))
    d.polygon(scale(star_points(160, 78, 34, 14, 5)), fill=(253, 224, 71, 255))
    return glow(img, 5)


def s_sparkle_ring():
    img, d = canvas()
    d.ellipse([46 * SS, 46 * SS, 210 * SS, 210 * SS], outline=(250, 204, 21, 220),
              width=7 * SS)
    d.polygon(scale(star_points(128, 128, 56, 13, 4)), fill=(253, 224, 71, 255))
    return glow(img, 6)


SPARKLES = [
    ('sparkle', 'Sparkle', s_sparkle4),
    ('sparkle-small', 'Small Sparkle', s_sparkle_small),
    ('star5', 'Five Point Star', s_star5),
    ('star8', 'Eight Point Star', s_star8),
    ('star-outline', 'Star Outline', s_star_outline),
    ('shooting-star', 'Shooting Star', s_shooting),
    ('sparkle-trail', 'Sparkle Trail', s_trail),
    ('glow-dot', 'Glow Dot', s_glow_dot),
    ('twinkle', 'Twinkle', s_twinkle),
    ('star-stack', 'Stacked Stars', s_star_stack),
    ('sparkle-ring', 'Sparkle Ring', s_sparkle_ring),
]


# --------------------------------------------------------------------------
# 5. Speech and labels
# --------------------------------------------------------------------------
def b_speech():
    img, d = canvas()
    d.rounded_rectangle([34 * SS, 46 * SS, 222 * SS, 170 * SS], radius=32 * SS,
                        fill=(255, 255, 255, 255))
    d.polygon(scale([(76, 164), (76, 216), (126, 166)]), fill=(255, 255, 255, 255))
    return with_shadow(img)


def b_speech_round():
    img, d = canvas()
    d.ellipse([34 * SS, 40 * SS, 222 * SS, 196 * SS], fill=(255, 255, 255, 255))
    d.polygon(scale([(92, 188), (88, 234), (140, 190)]), fill=(255, 255, 255, 255))
    return with_shadow(img)


def b_thought():
    img, d = canvas()
    d.ellipse([40 * SS, 38 * SS, 216 * SS, 172 * SS], outline=(255, 255, 255, 255),
              width=11 * SS)
    d.ellipse([96 * SS, 186 * SS, 128 * SS, 214 * SS], fill=(255, 255, 255, 255))
    d.ellipse([116 * SS, 222 * SS, 138 * SS, 242 * SS], fill=(255, 255, 255, 220))
    return img


def b_shout():
    img, d = canvas()
    pts = []
    for i in range(24):
        a = i * 2 * math.pi / 24
        r = 100 if i % 2 == 0 else 72
        pts.append((128 + r * math.cos(a), 118 + r * math.sin(a)))
    d.polygon(scale(pts), fill=(250, 204, 21, 255))
    return with_shadow(img)


def b_comic():
    img, d = canvas()
    pts = []
    for i in range(20):
        a = i * 2 * math.pi / 20
        r = 104 if i % 2 == 0 else 76
        pts.append((128 + r * math.cos(a), 128 + r * math.sin(a)))
    d.polygon(scale(pts), fill=(236, 72, 153, 255))
    d.polygon(scale(pts), outline=(255, 255, 255, 255), width=8 * SS)
    return with_shadow(img)


def b_exclaim():
    img, d = canvas()
    d.rounded_rectangle([46 * SS, 52 * SS, 210 * SS, 168 * SS], radius=30 * SS,
                        fill=(37, 99, 235, 255))
    d.rounded_rectangle([112 * SS, 78 * SS, 144 * SS, 128 * SS], radius=8 * SS,
                        fill=(255, 255, 255, 255))
    d.ellipse([112 * SS, 140 * SS, 144 * SS, 172 * SS], fill=(255, 255, 255, 255))
    return with_shadow(img)


def b_question():
    img, d = canvas()
    d.rounded_rectangle([46 * SS, 52 * SS, 210 * SS, 168 * SS], radius=30 * SS,
                        fill=(168, 85, 247, 255))
    d.ellipse([108 * SS, 136 * SS, 148 * SS, 176 * SS], fill=(255, 255, 255, 255))
    d.arc([106 * SS, 66 * SS, 152 * SS, 130 * SS], 160, 20, fill=(255, 255, 255, 255),
          width=13 * SS)
    return with_shadow(img)


def b_heart_bubble():
    img, d = canvas()
    d.rounded_rectangle([36 * SS, 48 * SS, 220 * SS, 164 * SS], radius=30 * SS,
                        fill=(244, 63, 94, 255))
    d.polygon(scale([(78, 158), (78, 210), (126, 162)]), fill=(244, 63, 94, 255))
    d.polygon(scale(heart_path(128, 104, 92, 88)), fill=(255, 255, 255, 255))
    return with_shadow(img)


def b_whisper():
    img, d = canvas()
    d.rounded_rectangle([52 * SS, 60 * SS, 204 * SS, 176 * SS], radius=40 * SS,
                        fill=(15, 23, 42, 220), outline=(255, 255, 255, 220),
                        width=5 * SS)
    for i, x in enumerate((92, 128, 164)):
        d.ellipse([(x - 10) * SS, 100 * SS, (x + 10) * SS, 120 * SS],
                  fill=(255, 255, 255, 255 - i * 50))
    return with_shadow(img)


def b_ribbon():
    img, d = canvas()
    d.polygon(scale([(46, 62), (210, 62), (210, 146), (128, 120), (46, 146)]),
              fill=(34, 197, 94, 255))
    d.polygon(scale([(46, 146), (96, 146), (70, 196)]), fill=(22, 163, 74, 255))
    d.polygon(scale([(210, 146), (160, 146), (186, 196)]), fill=(22, 163, 74, 255))
    return with_shadow(img)


def b_tag():
    img, d = canvas()
    d.polygon(scale([(50, 74), (150, 74), (210, 128), (150, 182), (50, 182)]),
              fill=(239, 68, 68, 255))
    d.ellipse([70 * SS, 112 * SS, 96 * SS, 138 * SS], fill=(255, 255, 255, 255))
    return with_shadow(img)


def b_label():
    img, d = canvas()
    d.rounded_rectangle([36 * SS, 82 * SS, 220 * SS, 174 * SS], radius=18 * SS,
                        fill=(59, 130, 246, 255))
    d.polygon(scale([(150, 174), (214, 196), (150, 210)]), fill=(37, 99, 235, 255))
    return with_shadow(img)


SPEECH = [
    ('speech', 'Speech Bubble', b_speech),
    ('speech-round', 'Round Bubble', b_speech_round),
    ('thought', 'Thought Bubble', b_thought),
    ('shout', 'Shout Burst', b_shout),
    ('comic-burst', 'Comic Burst', b_comic),
    ('bubble-exclaim', 'Exclamation Bubble', b_exclaim),
    ('bubble-question', 'Question Bubble', b_question),
    ('bubble-heart', 'Heart Bubble', b_heart_bubble),
    ('whisper', 'Whisper', b_whisper),
    ('ribbon', 'Ribbon', b_ribbon),
    ('tag', 'Tag', b_tag),
    ('label', 'Label', b_label),
]


# --------------------------------------------------------------------------
# 6. Badges and markers
# --------------------------------------------------------------------------
def badge_base(d, fill, edge):
    d.ellipse([34 * SS, 34 * SS, 222 * SS, 222 * SS], fill=fill)
    d.ellipse([34 * SS, 34 * SS, 222 * SS, 222 * SS], outline=edge, width=8 * SS)


def bd_new():
    img, d = canvas()
    badge_base(d, (239, 68, 68, 255), (255, 255, 255, 255))
    d.polygon(scale(star_points(186, 70, 40, 16, 5)), fill=(250, 204, 21, 255))
    return with_shadow(img)


def bd_hot():
    img, d = canvas()
    pts = []
    for i in range(20):
        a = i * 2 * math.pi / 20
        r = 100 if i % 2 == 0 else 74
        pts.append((128 + r * math.cos(a), 128 + r * math.sin(a)))
    d.polygon(scale(pts), fill=(249, 115, 22, 255))
    d.polygon(scale(star_points(128, 128, 46, 20, 5)), fill=(254, 240, 138, 255))
    return with_shadow(img)


def bd_sale():
    img, d = canvas()
    d.rounded_rectangle([36 * SS, 78 * SS, 220 * SS, 178 * SS], radius=20 * SS,
                        fill=(244, 63, 94, 255))
    d.ellipse([58 * SS, 100 * SS, 198 * SS, 156 * SS], outline=(255, 255, 255, 255),
              width=7 * SS)
    return with_shadow(img)


def bd_check():
    img, d = canvas()
    badge_base(d, (34, 197, 94, 255), (255, 255, 255, 255))
    d.line(scale([(84, 130), (116, 162), (176, 96)]), fill=(255, 255, 255, 255),
           width=16 * SS, joint='curve')
    return with_shadow(img)


def bd_cross():
    img, d = canvas()
    badge_base(d, (220, 38, 38, 255), (255, 255, 255, 255))
    w = 14 * SS
    d.line(scale([(96, 96), (160, 160)]), fill=(255, 255, 255, 255), width=w)
    d.line(scale([(160, 96), (96, 160)]), fill=(255, 255, 255, 255), width=w)
    return with_shadow(img)


def bd_warning():
    img, d = canvas()
    d.polygon(scale([(128, 36), (226, 210), (30, 210)]), fill=(250, 204, 21, 255))
    d.polygon(scale([(128, 36), (226, 210), (30, 210)]), outline=(120, 53, 15, 255),
              width=8 * SS)
    d.rounded_rectangle([116 * SS, 106 * SS, 140 * SS, 158 * SS], radius=6 * SS,
                        fill=(120, 53, 15, 255))
    d.ellipse([116 * SS, 172 * SS, 140 * SS, 196 * SS], fill=(120, 53, 15, 255))
    return with_shadow(img)


def bd_crown():
    img, d = canvas()
    d.polygon(scale([(44, 186), (44, 96), (96, 140), (128, 74), (160, 140),
                     (212, 96), (212, 186)]), fill=(250, 204, 21, 255))
    d.rectangle([44 * SS, 186 * SS, 212 * SS, 208 * SS], fill=(202, 138, 4, 255))
    for x in (44, 128, 212):
        d.ellipse([(x - 12) * SS, 84 * SS, (x + 12) * SS, 108 * SS],
                  fill=(239, 68, 68, 255))
    return with_shadow(img)


def bd_percent():
    img, d = canvas()
    d.rounded_rectangle([36 * SS, 70 * SS, 220 * SS, 186 * SS], radius=24 * SS,
                        fill=(168, 85, 247, 255))
    d.ellipse([72 * SS, 96 * SS, 104 * SS, 128 * SS], fill=(255, 255, 255, 255))
    d.ellipse([152 * SS, 128 * SS, 184 * SS, 160 * SS], fill=(255, 255, 255, 255))
    d.line(scale([(84, 156), (172, 104)]), fill=(255, 255, 255, 255), width=12 * SS)
    return with_shadow(img)


def bd_ribbon_circle():
    img, d = canvas()
    d.polygon(scale([(64, 152), (64, 226), (128, 192), (192, 226), (192, 152)]),
              fill=(220, 38, 38, 255))
    badge_base(d, (239, 68, 68, 255), (255, 255, 255, 255))
    return with_shadow(img)


def bd_target():
    img, d = canvas()
    for r, c in ((92, (239, 68, 68)), (62, (255, 255, 255)), (32, (239, 68, 68))):
        d.ellipse([(128 - r) * SS, (128 - r) * SS, (128 + r) * SS, (128 + r) * SS],
                  fill=c + (255,))
    return with_shadow(img)


def bd_shield():
    img, d = canvas()
    d.polygon(scale([(128, 30), (218, 62), (218, 130), (128, 226), (38, 130),
                     (38, 62)]), fill=(37, 99, 235, 255))
    d.line(scale([(92, 126), (118, 154), (168, 98)]), fill=(255, 255, 255, 255),
           width=14 * SS, joint='curve')
    return with_shadow(img)


BADGES = [
    ('badge-new', 'New Badge', bd_new),
    ('badge-hot', 'Hot Badge', bd_hot),
    ('badge-sale', 'Sale Badge', bd_sale),
    ('badge-check', 'Check Badge', bd_check),
    ('badge-cross', 'Cross Badge', bd_cross),
    ('badge-warning', 'Warning', bd_warning),
    ('crown', 'Crown', bd_crown),
    ('percent', 'Percent', bd_percent),
    ('ribbon-circle', 'Ribbon Badge', bd_ribbon_circle),
    ('target', 'Target', bd_target),
    ('shield', 'Shield Check', bd_shield),
]


# --------------------------------------------------------------------------
# 7. Frames and marks
# --------------------------------------------------------------------------
def f_film():
    img, d = canvas()
    d.rounded_rectangle([26 * SS, 74 * SS, 230 * SS, 182 * SS], radius=14 * SS,
                        outline=(255, 255, 255, 255), width=9 * SS)
    for x in range(46, 220, 36):
        d.rectangle([x * SS, 82 * SS, (x + 18) * SS, 100 * SS],
                    fill=(255, 255, 255, 235))
        d.rectangle([x * SS, 156 * SS, (x + 18) * SS, 174 * SS],
                    fill=(255, 255, 255, 235))
    return with_shadow(img)


def f_polaroid():
    img, d = canvas()
    d.rectangle([40 * SS, 46 * SS, 216 * SS, 222 * SS], fill=(255, 255, 255, 245))
    d.rectangle([54 * SS, 60 * SS, 202 * SS, 178 * SS], fill=(30, 41, 59, 255))
    return with_shadow(img)


def f_torn():
    img, d = canvas()
    pts = []
    n = 26
    for i in range(n):
        a = i * 2 * math.pi / n
        r = 96 + (16 if i % 2 else -14) * ((i * 7) % 3 + 1) / 3.0
        pts.append((128 + r * math.cos(a), 128 + r * math.sin(a)))
    d.polygon(scale(pts), outline=(255, 255, 255, 235), width=10 * SS)
    return img


def f_corners():
    img, d = canvas()
    w = 11 * SS
    for (x, y), (sx, sy) in (((52, 52), (1, 1)), ((204, 52), (-1, 1)),
                             ((52, 204), (1, -1)), ((204, 204), (-1, -1))):
        d.line(scale([(x, y), (x + sx * 40, y)]), fill=(255, 255, 255, 255), width=w)
        d.line(scale([(x, y), (x, y + sy * 40)]), fill=(255, 255, 255, 255), width=w)
    return img


def f_round_border():
    img, d = canvas()
    d.ellipse([34 * SS, 34 * SS, 222 * SS, 222 * SS], outline=(255, 255, 255, 255),
              width=10 * SS)
    return img


def f_rounded_border():
    img, d = canvas()
    d.rounded_rectangle([34 * SS, 44 * SS, 222 * SS, 212 * SS], radius=40 * SS,
                        outline=(255, 255, 255, 255), width=10 * SS)
    return img


def f_gradient_border():
    img, d = canvas()
    for i, col in enumerate((
            (244, 63, 94, 255), (251, 146, 60, 255), (250, 204, 21, 255),
            (34, 197, 94, 255), (59, 130, 246, 255), (168, 85, 247, 255))):
        pad = i * 7
        d.rounded_rectangle(
            [(34 + pad) * SS, (44 + pad) * SS, (222 - pad) * SS, (212 - pad) * SS],
            radius=max(4, (40 - pad)) * SS, outline=col, width=7 * SS)
    return img


def f_comic_border():
    img, d = canvas()
    for i in range(0, 30, 2):
        a0 = i * math.pi / 15
        d.line(scale([(128 + 84 * math.cos(a0), 128 + 84 * math.sin(a0)),
                      (128 + 110 * math.cos(a0), 128 + 110 * math.sin(a0))]),
               fill=(255, 255, 255, 235), width=10 * SS)
    d.ellipse([48 * SS, 48 * SS, 208 * SS, 208 * SS], outline=(255, 255, 255, 255),
              width=8 * SS)
    return img


def f_vintage():
    img, d = canvas()
    d.rounded_rectangle([30 * SS, 40 * SS, 226 * SS, 216 * SS], radius=10 * SS,
                        outline=(255, 245, 220, 255), width=8 * SS)
    d.rounded_rectangle([46 * SS, 56 * SS, 210 * SS, 200 * SS], radius=4 * SS,
                        outline=(255, 245, 220, 190), width=4 * SS)
    return img


def f_tape():
    img, d = canvas()
    for cx, cy, rot in ((66, 58, -30), (190, 58, 30), (66, 198, 30), (190, 198, -30)):
        d.polygon(scale([(cx - 34, cy - 14), (cx + 34, cy - 20),
                         (cx + 34, cy + 16), (cx - 34, cy + 20)]),
                  fill=(254, 240, 138, 190))
    d.rounded_rectangle([40 * SS, 60 * SS, 216 * SS, 196 * SS], radius=12 * SS,
                        outline=(255, 255, 255, 235), width=8 * SS)
    return img


def f_scribble():
    img, d = canvas()
    pts = []
    for i in range(160):
        a = i * 2 * math.pi / 160
        r = 92 + 12 * math.sin(a * 5)
        pts.append((128 + r * math.cos(a), 128 + r * math.sin(a)))
    d.line(scale(pts), fill=(255, 255, 255, 235), width=8 * SS, joint='curve')
    return img


FRAMES = [
    ('frame-film', 'Film Frame', f_film),
    ('frame-polaroid', 'Polaroid', f_polaroid),
    ('frame-torn', 'Torn Edge', f_torn),
    ('frame-corners', 'Corners', f_corners),
    ('frame-round', 'Round Border', f_round_border),
    ('frame-rounded', 'Rounded Border', f_rounded_border),
    ('frame-gradient', 'Gradient Border', f_gradient_border),
    ('frame-comic', 'Comic Border', f_comic_border),
    ('frame-vintage', 'Vintage Frame', f_vintage),
    ('frame-tape', 'Taped Frame', f_tape),
    ('frame-scribble', 'Scribble', f_scribble),
]


# --------------------------------------------------------------------------
# 8. Motion marks and symbols
# --------------------------------------------------------------------------
def m_speed_lines():
    img, d = canvas()
    for i, (y, x0, x1, w, a) in enumerate((
            (74, 60, 196, 12, 255), (104, 44, 176, 9, 210), (134, 60, 196, 12, 255),
            (164, 44, 176, 9, 210), (194, 60, 196, 12, 255))):
        d.line(scale([(x0, y), (x1, y)]), fill=(255, 255, 255, a),
               width=w * SS)
    return img


def m_swoosh():
    img, d = canvas()
    d.line(scale([(40, 186), (86, 96), (166, 76), (216, 108)]),
           fill=(255, 255, 255, 235), width=20 * SS, joint='curve')
    return img


def m_blur_streak():
    img, d = canvas()
    for i in range(6):
        a = 90 + i * 28
        d.line(scale([(50, 128 + i * 4), (206, 128 - i * 4)]),
               fill=(255, 255, 255, 70 + i * 26), width=(20 - i * 2) * SS)
    return img


def m_impact():
    img, d = canvas()
    for i in range(14):
        a = i * 2 * math.pi / 14
        d.line(scale([(128 + 56 * math.cos(a), 128 + 56 * math.sin(a)),
                      (128 + 96 * math.cos(a), 128 + 96 * math.sin(a))]),
               fill=(255, 255, 255, 240), width=9 * SS)
    return glow(img, 5)


def m_dust():
    img, d = canvas()
    for cx, cy, r in ((100, 150, 30), (152, 122, 22), (120, 96, 16), (176, 168, 18)):
        d.ellipse([(cx - r) * SS, (cy - r) * SS, (cx + r) * SS, (cy + r) * SS],
                  fill=(255, 255, 255, 175))
    return img


def m_lens():
    img, d = canvas()
    d.ellipse([70 * SS, 70 * SS, 186 * SS, 186 * SS], outline=(255, 255, 255, 235),
              width=9 * SS)
    d.line(scale([(46, 74), (210, 182)]), fill=(255, 255, 255, 235), width=9 * SS)
    return glow(img, 6)


def m_zigzag_motion():
    img, d = canvas()
    for i, off in enumerate((0, 34, 68)):
        d.line(scale([(40 + off, 196), (74 + off, 96), (108 + off, 178),
                      (142 + off, 86), (176 + off, 150)]),
               fill=(255, 255, 255, 255 - i * 70), width=10 * SS, joint='curve')
    return img


def y_check():
    img, d = canvas()
    d.line(scale([(56, 132), (110, 188), (204, 70)]), fill=(34, 197, 94, 255),
           width=26 * SS, joint='curve')
    return with_shadow(img)


def y_cross():
    img, d = canvas()
    w = 24 * SS
    d.line(scale([(66, 66), (190, 190)]), fill=(220, 38, 38, 255), width=w)
    d.line(scale([(190, 66), (66, 190)]), fill=(220, 38, 38, 255), width=w)
    return with_shadow(img)


def y_plus():
    img, d = canvas()
    w = 22 * SS
    d.line(scale([(52, 128), (204, 128)]), fill=(59, 130, 246, 255), width=w)
    d.line(scale([(128, 52), (128, 204)]), fill=(59, 130, 246, 255), width=w)
    return with_shadow(img)


def y_infinity():
    img, d = canvas()
    d.ellipse([28 * SS, 88 * SS, 132 * SS, 168 * SS], outline=(236, 72, 153, 255),
              width=18 * SS)
    d.ellipse([124 * SS, 88 * SS, 228 * SS, 168 * SS], outline=(236, 72, 153, 255),
              width=18 * SS)
    return img


def y_question():
    img, d = canvas()
    d.arc([84 * SS, 52 * SS, 172 * SS, 150 * SS], 150, 30,
          fill=(124, 58, 237, 255), width=20 * SS)
    d.ellipse([114 * SS, 178 * SS, 142 * SS, 206 * SS], fill=(124, 58, 237, 255))
    return with_shadow(img)


def y_exclaim():
    img, d = canvas()
    d.rounded_rectangle([112 * SS, 44 * SS, 144 * SS, 164 * SS], radius=9 * SS,
                        fill=(250, 204, 21, 255))
    d.ellipse([112 * SS, 182 * SS, 144 * SS, 214 * SS], fill=(250, 204, 21, 255))
    return with_shadow(img)


def y_hashtag():
    img, d = canvas()
    w = 14 * SS
    for x in (96, 152):
        d.line(scale([(x, 52), (x - 12, 204)]), fill=(37, 99, 235, 255), width=w)
    for y in (104, 156):
        d.line(scale([(52, y), (204, y)]), fill=(37, 99, 235, 255), width=w)
    return with_shadow(img)


def y_at():
    img, d = canvas()
    d.ellipse([56 * SS, 56 * SS, 200 * SS, 200 * SS], outline=(16, 185, 129, 255),
              width=16 * SS)
    d.ellipse([98 * SS, 98 * SS, 158 * SS, 158 * SS], outline=(16, 185, 129, 255),
              width=14 * SS)
    d.line(scale([(158, 128), (158, 176)]), fill=(16, 185, 129, 255), width=14 * SS)
    return img


def y_droplet():
    img, d = canvas()
    d.ellipse([72 * SS, 108 * SS, 184 * SS, 208 * SS], fill=(56, 189, 248, 255))
    d.polygon(scale([(128, 36), (184, 126), (72, 126)]), fill=(56, 189, 248, 255))
    return with_shadow(img)


def y_leaf():
    """Two mirrored quadratic arcs meeting at a tip and a stem point."""
    img = Image.new('RGBA', (SIZE * SS, SIZE * SS), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    S = SIZE * SS
    tip, base = (46 * S / 256.0, 212 * S / 256.0), (210 * S / 256.0, 44 * S / 256.0)
    cx, cy = 128 * S / 256.0, 128 * S / 256.0
    pts = []
    for side in (1, -1):
        rng = range(0, 41) if side == 1 else range(40, -1, -1)
        for i in rng:
            t = i / 40.0
            # quadratic bezier tip -> control -> base
            bx = (1 - t) ** 2 * tip[0] + 2 * (1 - t) * t * cx + t ** 2 * base[0]
            by = (1 - t) ** 2 * tip[1] + 2 * (1 - t) * t * cy + t ** 2 * base[1]
            nx, ny = base[0] - cx, base[1] - cy
            ln = math.hypot(nx, ny) or 1.0
            bulge = 56 * math.sin(math.pi * t) * side
            pts.append((bx - ny / ln * bulge, by + nx / ln * bulge))
    d.polygon(pts, fill=(34, 197, 94, 255))
    d.line([tip, (cx * 0.86 + base[0] * 0.14, cy * 0.86 + base[1] * 0.14)],
           fill=(255, 255, 255, 210), width=max(2, int(6 * SS / 4)))
    return with_shadow(img)


def y_cloud():
    img, d = canvas()
    d.ellipse([52 * SS, 108 * SS, 128 * SS, 176 * SS], fill=(255, 255, 255, 240))
    d.ellipse([96 * SS, 76 * SS, 180 * SS, 176 * SS], fill=(255, 255, 255, 240))
    d.ellipse([146 * SS, 112 * SS, 212 * SS, 176 * SS], fill=(255, 255, 255, 240))
    d.rectangle([70 * SS, 140 * SS, 196 * SS, 176 * SS], fill=(255, 255, 255, 240))
    return with_shadow(img)


def y_flower():
    img, d = canvas()
    for i in range(6):
        a = i * math.pi / 3
        cx, cy = 128 + 46 * math.cos(a), 128 + 46 * math.sin(a)
        d.ellipse([(cx - 32) * SS, (cy - 32) * SS, (cx + 32) * SS, (cy + 32) * SS],
                  fill=(244, 114, 182, 255))
    d.ellipse([96 * SS, 96 * SS, 160 * SS, 160 * SS], fill=(250, 204, 21, 255))
    return with_shadow(img)


def y_globe():
    img, d = canvas()
    d.ellipse([40 * SS, 40 * SS, 216 * SS, 216 * SS], outline=(56, 189, 248, 255),
              width=10 * SS)
    d.line(scale([(40, 128), (216, 128)]), fill=(56, 189, 248, 255), width=8 * SS)
    d.line(scale([(128, 40), (128, 216)]), fill=(56, 189, 248, 255), width=8 * SS)
    d.ellipse([88 * SS, 40 * SS, 168 * SS, 216 * SS], outline=(56, 189, 248, 255),
              width=8 * SS)
    return img


MARKS = [
    ('speed-lines', 'Speed Lines', m_speed_lines),
    ('swoosh', 'Swoosh', m_swoosh),
    ('blur-streak', 'Blur Streak', m_blur_streak),
    ('impact', 'Impact', m_impact),
    ('dust', 'Dust', m_dust),
    ('lens', 'Lens Flare', m_lens),
    ('zigzag-motion', 'Zigzag Motion', m_zigzag_motion),
    ('sym-check', 'Check', y_check),
    ('sym-cross', 'X', y_cross),
    ('sym-plus', 'Plus', y_plus),
    ('sym-infinity', 'Infinity', y_infinity),
    ('sym-question', 'Question Mark', y_question),
    ('sym-exclaim', 'Exclamation Mark', y_exclaim),
    ('sym-hashtag', 'Hashtag', y_hashtag),
    ('sym-at', 'At Sign', y_at),
    ('droplet', 'Droplet', y_droplet),
    ('leaf', 'Leaf', y_leaf),
    ('cloud', 'Cloud', y_cloud),
    ('flower', 'Flower', y_flower),
    ('globe', 'Globe', y_globe),
]

# The eight categories the tab filters on, in display order.
GROUPS = [
    ('shape', SHAPES),
    ('heart', HEARTS),
    ('arrow', ARROWS),
    ('sparkle', SPARKLES),
    ('speech', SPEECH),
    ('badge', BADGES),
    ('frame', FRAMES),
    ('mark', MARKS),
]
ALL = [(gid, name, cat, fn) for cat, entries in GROUPS for gid, name, fn in entries]


def main():
    os.makedirs(OUT, exist_ok=True)
    for sid, name, cat, fn in ALL:
        render(fn, sid, name, cat)
    total = sum(os.path.getsize(os.path.join(OUT, f)) for f in os.listdir(OUT))
    print('%d stickers -> %s  (%.0f KB total, %.0f KB avg)'
          % (len(CATALOG), OUT, total / 1024, total / 1024 / len(CATALOG)))

    # Emit the Rust catalog so the two cannot drift.
    lines = ['//! Built-in sticker library. Generated -- do not hand-edit.',
             '//!',
             '//! Produced by `crates/app/assets/gen_stickers.py`. Every id here',
             '//! has a PNG beside this file and is embedded in the binary, so a',
             '//! fresh install has the whole library with no download step.',
             '',
             '/// One sticker: a stable id, a display name, and the file bytes.',
             '#[derive(Debug, Clone, Copy, PartialEq, Eq)]',
             'pub struct Sticker {',
             '    pub id: &\'static str,',
             '    pub name: &\'static str,',
             '    pub category: &\'static str,',
             '    pub png: &\'static [u8],',
             '}',
             '',
             'pub const CATALOG: &[Sticker] = &[']
    def q(x):
        # A Rust string literal, not Python's repr.
        return json.dumps(x, ensure_ascii=False)

    for sid, name, cat in CATALOG:
        lines.append('    Sticker {')
        lines.append('        id: %s,' % q(sid))
        lines.append('        name: %s,' % q(name))
        lines.append('        category: %s,' % q(cat))
        lines.append('        png: include_bytes!("../../app/assets/stickers/%s.png"),' % sid)
        lines.append('    },')
    lines.append('];')
    lines.append('')
    src = os.path.join(os.path.dirname(OUT), '..', '..', 'core', 'src', 'stickers.rs')
    # Everything above MARKER is regenerated; everything below it is
    # hand-written and survives. Without the split, regenerating the
    # catalog would silently delete the helper functions and tests that
    # live in the same file.
    marker = '// ---- end of generated section ----'
    below = ''
    if os.path.exists(src):
        with open(src, encoding='utf-8') as fh:
            existing = fh.read()
        if marker in existing:
            below = marker + existing.split(marker, 1)[1]
    lines.append(below.lstrip('\n'))
    lines.append(marker)
    src = os.path.normpath(src)
    with open(src, 'w', encoding='utf-8', newline='\n') as fh:
        fh.write('\n'.join(lines))
    print('catalog -> %s' % src)


if __name__ == '__main__':
    main()

