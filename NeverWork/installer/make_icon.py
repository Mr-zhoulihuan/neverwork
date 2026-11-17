# -*- coding: utf-8 -*-
"""生成 NeverWork 应用图标（多尺寸 ICO）。

品牌绿 #22C55E 与左侧栏头像一致；白字 N 居中。
产物写入 crates/app/assets/neverwork.ico（已入库）：app 的 build.rs 用
embed-resource 把它内嵌进 exe（任务栏 / 资源管理器图标），NSIS 安装包也直接引用。
修改品牌后手动重新生成：python installer\make_icon.py
"""
import os

from PIL import Image, ImageDraw, ImageFont

GREEN = (34, 197, 94, 255)  # #22C55E
WHITE = (255, 255, 255, 255)
HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.normpath(os.path.join(HERE, "..", "crates", "app", "assets", "neverwork.ico"))
FONTS = ("seguisb.ttf", "segoeuib.ttf", "arialbd.ttf")


def make(size: int) -> Image.Image:
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)
    radius = int(size * 0.22)
    draw.rounded_rectangle([0, 0, size - 1, size - 1], radius=radius, fill=GREEN)
    font = None
    for name in FONTS:
        try:
            font = ImageFont.truetype(
                os.path.join(os.environ["WINDIR"], "Fonts", name), int(size * 0.64)
            )
            break
        except OSError:
            continue
    if font is None:
        font = ImageFont.load_default()
    draw.text((size / 2, size / 2), "N", font=font, fill=WHITE, anchor="mm")
    return img


def main() -> None:
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    base = make(256)
    sizes = [(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)]
    base.save(OUT, format="ICO", sizes=sizes)
    print("icon:", OUT)


if __name__ == "__main__":
    main()
