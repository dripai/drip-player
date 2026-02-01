import os
import sys
from PIL import Image, ImageDraw, ImageFont

def create_icon_from_text(text, output_dir, bg_color=(30, 30, 45)):
    """
    根据文本生成图标文件 (ICO, PNG, ICNS)。
    使用 PIL 库绘制图像。
    
    参数:
        text: 图标上显示的文字
        output_dir: 输出目录
        bg_color: 背景颜色 (R, G, B) 元组或颜色字符串
    """
    # 确保输出目录存在
    if not os.path.exists(output_dir):
        os.makedirs(output_dir)

    # 基础图像尺寸 (使用最大的 512x512 作为基准，保证清晰度)
    base_size = (512, 512)
    text_color = (255, 255, 255) # 白色文字

    # 创建新图像 RGBA (全透明背景)
    img = Image.new('RGBA', base_size, (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)

    # 绘制圆角矩形背景
    # 圆角半径，设为宽度的 20% 左右
    radius = int(base_size[0] * 0.2)
    rect = [0, 0, base_size[0], base_size[1]]
    
    try:
        # Pillow >= 8.2.0 支持 rounded_rectangle
        draw.rounded_rectangle(rect, radius=radius, fill=bg_color)
    except AttributeError:
        # 兼容旧版本 Pillow: 手动绘制圆角矩形
        # 简化处理：如果不支持，画一个普通矩形
        print("警告: 当前 PIL 版本不支持 rounded_rectangle，将生成方形图标。")
        draw.rectangle(rect, fill=bg_color)

    # 尝试加载字体
    font = None
    try:
        # Windows 常见字体路径
        # 尝试 Arial Bold (通常存在)
        font_path = "C:\\Windows\\Fonts\\arialbd.ttf" 
        if not os.path.exists(font_path):
            font_path = "C:\\Windows\\Fonts\\arial.ttf"
        
        # 根据图像大小设置字体大小 
        # 用户要求放大文字: 从原来的 0.47 增加到 0.75 左右
        font_size = int(base_size[1] * 0.75)
        font = ImageFont.truetype(font_path, font_size)
    except Exception as e:
        print(f"警告: 无法加载系统字体 ({e})，使用默认字体。")
        font = ImageFont.load_default()

    # 计算文本位置使其居中
    try:
        left, top, right, bottom = draw.textbbox((0, 0), text, font=font)
        text_width = right - left
        text_height = bottom - top
        # textbbox 的 offset
        offset_x = left
        offset_y = top
    except AttributeError:
        # 兼容旧版本 Pillow
        text_width, text_height = draw.textsize(text, font=font)
        offset_x = 0
        offset_y = 0

    x = (base_size[0] - text_width) / 2 - offset_x
    y = (base_size[1] - text_height) / 2 - offset_y
    
    # 稍微调整 y 轴以视觉居中 (减去一些顶部偏移，因为字体基线问题)
    # 字体变大后，通常需要微调垂直位置
    # 用户反馈偏上，需要往下来点，改为增加偏移
    y = y + (text_height * 0.08) 

    draw.text((x, y), text, font=font, fill=text_color)

    # 1. 生成 PNG 图片 (循环处理不同尺寸)
    # 定义需要的尺寸和对应的文件名
    # 32x32, 128x128, 192x192, 512x512
    png_sizes = {
        32: "32x32.png",
        128: "128x128.png",
        192: "icon-192.png",
        512: "icon-512.png"
    }

    for size, filename in png_sizes.items():
        output_path = os.path.join(output_dir, filename)
        # 使用 Lanczos 滤镜进行高质量缩放
        resized_img = img.resize((size, size), Image.Resampling.LANCZOS)
        resized_img.save(output_path, format='PNG')
        print(f"生成 PNG 文件 ({size}x{size}): {output_path}")

    # 额外保存 icon.png (通常用作主图标，使用 512x512 或与 icon-512.png 相同)
    icon_png_path = os.path.join(output_dir, "icon.png")
    img.save(icon_png_path, format='PNG')
    print(f"生成主 PNG 文件: {icon_png_path}")

    # 2. 保存为 ICO (包含多种尺寸)
    ico_path = os.path.join(output_dir, "icon.ico")
    # ICO 格式通常需要包含多种尺寸以便系统在不同视图下使用
    # 注意：Windows ICO 最大通常支持到 256x256
    img.save(ico_path, format='ICO', sizes=[(256, 256), (128, 128), (64, 64), (48, 48), (32, 32), (16, 16)])
    print(f"生成 ICO 文件: {ico_path}")
    
    # 3. 保存为 ICNS (Mac)
    icns_path = os.path.join(output_dir, "icon.icns")
    try:
        img.save(icns_path, format='ICNS')
        print(f"生成 ICNS 文件: {icns_path}")
    except:
        # 如果不支持 ICNS 写入，回退到写入 PNG 内容 (同原脚本逻辑，作为占位)
        img.save(icns_path, format='PNG')
        print(f"生成 ICNS 文件 (PNG内容占位): {icns_path}")

if __name__ == "__main__":
    # 配置参数
    text_to_draw = "D"
    output_directory = "src-tauri/icons"
    # 背景颜色 (R, G, B) - 可以在这里修改
    background_color = "blue" 
    
    print(f"正在生成基于文本 '{text_to_draw}' 的图标...")
    print(f"背景颜色: {background_color}")
    
    create_icon_from_text(text_to_draw, output_directory, bg_color=background_color)
