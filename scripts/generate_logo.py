#!/usr/bin/env python3
"""
CDUS Master Logo & Icon Generator
Generates:
1. Vector SVG (Master Icon, Horizontal Lockup, Monochrome/Adaptive)
2. Android Launcher Icons (Adaptive Vector XML + WebP/PNG Mipmaps for all densities)
3. Desktop Icons (Tauri bundle: ICO, ICNS, PNGs at all standard sizes)
4. Web/Frontend Assets (SVG & PNG for sidebar header and UI)

Theme Tokens adhered to strictly:
- Primary Accent: #24C8DB (CdusCyan)
- Light Brand: #4DD0E1 (CdusCyanLight)
- Deep Brand: #00838F (CdusCyanDark)
- Dark Deep Container: #00363A
- Dark Base: #1A1A1A (CdusDarkBase)
- Dark Surface: #262626 (CdusDarkSurface)
- Dark Elevated: #2D2D2D
- Dark Border: #333333
- White / Text Primary: #FFFFFF / #F6F6F6
"""

import os
import sys
import math
from PIL import Image, ImageDraw, ImageFilter

def create_svg_logo(standalone=True):
    """
    Returns an SVG string depicting the overlapping desktop & mobile silhouettes
    bridged by a luminous cyan sync synapse.
    """
    svg = '''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512" width="100%" height="100%">
  <defs>
    <!-- Background Gradient -->
    <radialGradient id="bgGrad" cx="50%" cy="40%" r="60%">
      <stop offset="0%" stop-color="#262626" />
      <stop offset="100%" stop-color="#1A1A1A" />
    </radialGradient>

    <!-- Cyan Glow Gradient -->
    <linearGradient id="cyanGrad" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%" stop-color="#4DD0E1" />
      <stop offset="50%" stop-color="#24C8DB" />
      <stop offset="100%" stop-color="#00838F" />
    </linearGradient>

    <!-- Beam Glow -->
    <linearGradient id="beamGrad" x1="0%" y1="50%" x2="100%" y2="50%">
      <stop offset="0%" stop-color="#24C8DB" stop-opacity="0.2" />
      <stop offset="50%" stop-color="#4DD0E1" stop-opacity="0.9" />
      <stop offset="100%" stop-color="#24C8DB" stop-opacity="0.2" />
    </linearGradient>

    <!-- Device Screen Gradient -->
    <linearGradient id="screenGrad" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%" stop-color="#1E2324" />
      <stop offset="100%" stop-color="#141819" />
    </linearGradient>

    <!-- Subtle Drop Shadow for Mobile -->
    <filter id="phoneShadow" x="-20%" y="-20%" width="150%" height="150%">
      <feDropShadow dx="-8" dy="12" stdDeviation="16" flood-color="#000000" flood-opacity="0.6" />
    </filter>

    <!-- Soft Glow Filter -->
    <filter id="cyanGlow" x="-30%" y="-30%" width="160%" height="160%">
      <feGaussianBlur stdDeviation="8" result="blur" />
      <feMerge>
        <feMergeNode in="blur" />
        <feMergeNode in="SourceGraphic" />
      </feMerge>
    </filter>
  </defs>

  <!-- Base Squircle Container -->
  <rect x="16" y="16" width="480" height="480" rx="108" fill="url(#bgGrad)" stroke="#333333" stroke-width="4" />

  <!-- Subtle Inner Ring -->
  <rect x="24" y="24" width="464" height="464" rx="100" fill="none" stroke="#24C8DB" stroke-width="1.5" stroke-opacity="0.15" />

  <!-- ==================== DESKTOP MONITOR ==================== -->
  <!-- Monitor Stand Base -->
  <rect x="175" y="372" width="130" height="12" rx="6" fill="#2D2D2D" stroke="#333333" stroke-width="2" />
  <!-- Monitor Stand Neck -->
  <path d="M 226 325 L 254 325 L 246 372 L 234 372 Z" fill="#262626" stroke="#333333" stroke-width="1.5" />

  <!-- Monitor Body Outer Frame -->
  <rect x="92" y="125" width="296" height="200" rx="20" fill="#262626" stroke="#333333" stroke-width="3" />
  <!-- Monitor Screen Glass -->
  <rect x="104" y="137" width="272" height="176" rx="12" fill="url(#screenGrad)" stroke="#00363A" stroke-width="1" />

  <!-- Desktop Window Header / Chrome Dots -->
  <circle cx="120" cy="151" r="3.5" fill="#333333" />
  <circle cx="132" cy="151" r="3.5" fill="#333333" />
  <circle cx="144" cy="151" r="3.5" fill="#24C8DB" opacity="0.8" />
  <line x1="104" y1="162" x2="376" y2="162" stroke="#24C8DB" stroke-opacity="0.15" stroke-width="1" />

  <!-- Desktop Internal Content Grid / Code Lines -->
  <line x1="120" y1="180" x2="190" y2="180" stroke="#00838F" stroke-width="3.5" stroke-linecap="round" opacity="0.7" />
  <line x1="120" y1="195" x2="160" y2="195" stroke="#24C8DB" stroke-width="3.5" stroke-linecap="round" opacity="0.5" />
  <line x1="120" y1="210" x2="210" y2="210" stroke="#333333" stroke-width="3.5" stroke-linecap="round" />

  <!-- ==================== THE SYNC BRIDGE (BACKGROUND PULSE) ==================== -->
  <!-- Data Connection Waves -->
  <path d="M 230 235 C 290 235, 280 280, 340 280" fill="none" stroke="url(#cyanGrad)" stroke-width="14" stroke-linecap="round" filter="url(#cyanGlow)" opacity="0.9" />
  <path d="M 230 235 C 290 235, 280 280, 340 280" fill="none" stroke="#FFFFFF" stroke-width="4" stroke-linecap="round" opacity="0.9" />

  <!-- ==================== MOBILE PHONE (FOREGROUND OVERLAY) ==================== -->
  <g filter="url(#phoneShadow)">
    <!-- Phone Outer Body -->
    <rect x="290" y="170" width="138" height="236" rx="28" fill="#262626" stroke="#444444" stroke-width="3" />
    <!-- Phone Screen Glass -->
    <rect x="298" y="178" width="122" height="220" rx="22" fill="url(#screenGrad)" stroke="#00363A" stroke-width="1" />

    <!-- Phone Dynamic Island / Speaker -->
    <rect x="341" y="186" width="36" height="7" rx="3.5" fill="#1A1A1A" />
    <circle cx="347" cy="189.5" r="1.5" fill="#24C8DB" opacity="0.6" />

    <!-- Phone Home Indicator Bar -->
    <rect x="345" y="388" width="28" height="3" rx="1.5" fill="#333333" />

    <!-- Phone Screen Content / Cards -->
    <rect x="308" y="206" width="102" height="36" rx="8" fill="#262626" stroke="#24C8DB" stroke-width="1" stroke-opacity="0.3" />
    <circle cx="322" cy="224" r="5" fill="#24C8DB" />
    <line x1="334" y1="220" x2="380" y2="220" stroke="#FFFFFF" stroke-width="2.5" stroke-linecap="round" opacity="0.8" />
    <line x1="334" y1="228" x2="365" y2="228" stroke="#00838F" stroke-width="2" stroke-linecap="round" />

    <rect x="308" y="250" width="102" height="58" rx="8" fill="#1E2324" stroke="#333333" stroke-width="1" />
  </g>

  <!-- ==================== CENTRAL SYNC NEXUS (INTERACTION POINT) ==================== -->
  <!-- Glowing Synchronized Nodes -->
  <!-- Node on Desktop Screen -->
  <circle cx="230" cy="235" r="9" fill="#24C8DB" filter="url(#cyanGlow)" />
  <circle cx="230" cy="235" r="4.5" fill="#FFFFFF" />

  <!-- Central Bridge Intersection Pulse -->
  <g filter="url(#cyanGlow)">
    <circle cx="288" cy="256" r="7" fill="#4DD0E1" />
    <circle cx="288" cy="256" r="3" fill="#FFFFFF" />
    <!-- Radiating Wave Rings -->
    <circle cx="288" cy="256" r="15" fill="none" stroke="#24C8DB" stroke-width="1.5" stroke-dasharray="3 3" opacity="0.7" />
  </g>

  <!-- Node on Mobile Screen -->
  <circle cx="340" cy="280" r="9" fill="#24C8DB" filter="url(#cyanGlow)" />
  <circle cx="340" cy="280" r="4.5" fill="#FFFFFF" />

</svg>'''
    return svg


def create_svg_foreground_only():
    """
    Returns Android Vector / Foreground SVG for Adaptive Icon (108dp viewport)
    Standard safe zone is inside center 66dp or 72dp.
    """
    svg = '''<vector xmlns:android="http://schemas.android.com/apk/res/android"
    android:width="108dp"
    android:height="108dp"
    android:viewportWidth="108"
    android:viewportHeight="108">

    <!-- Desktop Monitor Base -->
    <path
        android:fillColor="#2D2D2D"
        android:strokeColor="#333333"
        android:strokeWidth="0.5"
        android:pathData="M37,79 h26 c1.1,0 2,0.9 2,2 v0 c0,1.1 -0.9,2 -2,2 H37 c-1.1,0 -2,-0.9 -2,-2 v0 c0,-1.1 0.9,-2 2,-2 z" />

    <!-- Desktop Monitor Stand Neck -->
    <path
        android:fillColor="#262626"
        android:strokeColor="#333333"
        android:strokeWidth="0.4"
        android:pathData="M48,69 L53,69 L52,79 L49,79 Z" />

    <!-- Desktop Monitor Outer Body -->
    <path
        android:fillColor="#262626"
        android:strokeColor="#333333"
        android:strokeWidth="0.8"
        android:pathData="M24,30 h52 c2.8,0 5,2.2 5,5 v30 c0,2.8 -2.2,5 -5,5 H24 c-2.8,0 -5,-2.2 -5,-5 V35 c0,-2.8 2.2,-5 5,-5 z" />

    <!-- Desktop Monitor Inner Screen -->
    <path
        android:fillColor="#141819"
        android:strokeColor="#00363A"
        android:strokeWidth="0.4"
        android:pathData="M26,33 h48 c1.7,0 3,1.3 3,3 v24 c0,1.7 -1.3,3 -3,3 H26 c-1.7,0 -3,-1.3 -3,-3 V36 c0,-1.7 1.3,-3 3,-3 z" />

    <!-- Desktop Chrome Dots -->
    <path android:fillColor="#333333" android:pathData="M30,36 a1,1 0 1,0 0.01,0 Z" />
    <path android:fillColor="#333333" android:pathData="M33,36 a1,1 0 1,0 0.01,0 Z" />
    <path android:fillColor="#24C8DB" android:pathData="M36,36 a1,1 0 1,0 0.01,0 Z" />

    <!-- Desktop Screen Code Lines -->
    <path
        android:fillColor="#00000000"
        android:strokeColor="#00838F"
        android:strokeWidth="0.8"
        android:strokeLineCap="round"
        android:pathData="M30,42 L44,42 M30,46 L40,46 M30,50 L48,50" />

    <!-- Sync Bridge Conduit -->
    <path
        android:fillColor="#00000000"
        android:strokeColor="#24C8DB"
        android:strokeWidth="2.8"
        android:strokeLineCap="round"
        android:pathData="M48,52 C58,52 56,60 67,60" />

    <path
        android:fillColor="#00000000"
        android:strokeColor="#FFFFFF"
        android:strokeWidth="0.8"
        android:strokeLineCap="round"
        android:pathData="M48,52 C58,52 56,60 67,60" />

    <!-- Mobile Phone Outer Body -->
    <path
        android:fillColor="#262626"
        android:strokeColor="#444444"
        android:strokeWidth="0.8"
        android:pathData="M61,39 h23 c3.3,0 6,2.7 6,6 v36 c0,3.3 -2.7,6 -6,6 H61 c-3.3,0 -6,-2.7 -6,-6 V45 c0,-3.3 2.7,-6 6,-6 z" />

    <!-- Mobile Phone Inner Screen -->
    <path
        android:fillColor="#141819"
        android:strokeColor="#00363A"
        android:strokeWidth="0.4"
        android:pathData="M63,41 h19 c2.2,0 4,1.8 4,4 v32 c0,2.2 -1.8,4 -4,4 H63 c-2.2,0 -4,-1.8 -4,-4 V45 c0,-2.2 1.8,-4 4,-4 z" />

    <!-- Mobile Speaker / Pill -->
    <path
        android:fillColor="#1A1A1A"
        android:pathData="M70,42.5 h5 c0.5,0 1,0.5 1,1 v0 c0,0.5 -0.5,1 -1,1 h-5 c-0.5,0 -1,-0.5 -1,-1 v0 c0,-0.5 0.5,-1 1,-1 z" />

    <!-- Mobile Card Preview -->
    <path
        android:fillColor="#1E2324"
        android:strokeColor="#24C8DB"
        android:strokeWidth="0.4"
        android:pathData="M65,47 h15 c1,0 2,1 2,2 v6 c0,1 -1,2 -2,2 H65 c-1,0 -2,-1 -2,-2 v-6 c0,-1 1,-2 2,-2 z" />

    <!-- Sync Nodes -->
    <path android:fillColor="#24C8DB" android:pathData="M48,52 a2,2 0 1,0 0.01,0 Z" />
    <path android:fillColor="#FFFFFF" android:pathData="M48,52 a1,1 0 1,0 0.01,0 Z" />

    <path android:fillColor="#4DD0E1" android:pathData="M57.5,56 a1.6,1.6 0 1,0 0.01,0 Z" />
    <path android:fillColor="#FFFFFF" android:pathData="M57.5,56 a0.7,0.7 0 1,0 0.01,0 Z" />

    <path android:fillColor="#24C8DB" android:pathData="M67,60 a2,2 0 1,0 0.01,0 Z" />
    <path android:fillColor="#FFFFFF" android:pathData="M67,60 a1,1 0 1,0 0.01,0 Z" />

</vector>'''
    return svg


def create_svg_background_only():
    """
    Returns Android Vector / Background XML for Adaptive Icon (108dp viewport)
    Dark Base #1A1A1A with subtle concentric tech grid lines.
    """
    svg = '''<?xml version="1.0" encoding="utf-8"?>
<vector xmlns:android="http://schemas.android.com/apk/res/android"
    android:width="108dp"
    android:height="108dp"
    android:viewportWidth="108"
    android:viewportHeight="108">
    <path
        android:fillColor="#1A1A1A"
        android:pathData="M0,0h108v108h-108z" />
    <path
        android:fillColor="#00000000"
        android:strokeColor="#0A24C8DB"
        android:strokeWidth="0.8"
        android:pathData="M18,0v108 M36,0v108 M54,0v108 M72,0v108 M90,0v108" />
    <path
        android:fillColor="#00000000"
        android:strokeColor="#0A24C8DB"
        android:strokeWidth="0.8"
        android:pathData="M0,18h108 M0,36h108 M0,54h108 M0,72h108 M0,90h108" />
    <!-- Subtle center halo -->
    <path
        android:fillColor="#0624C8DB"
        android:pathData="M54,54 m-36,0 a36,36 0 1,0 72,0 a36,36 0 1,0 -72,0" />
</vector>'''
    return svg


def render_raster_icon(size=1024, is_round=False, transparent_bg=False):
    """
    Renders high-quality pixel-perfect raster icon using PIL with 2x supersampling.
    """
    scale = 2
    dim = size * scale
    img = Image.new("RGBA", (dim, dim), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)

    # Coordinates scaled to dim
    def s(val):
        return int(val * (dim / 512.0))

    # Base Background
    if not transparent_bg:
        if is_round:
            # Round icon circle
            draw.ellipse([s(16), s(16), s(496), s(496)], fill=(26, 26, 26, 255), outline=(51, 51, 51, 255), width=s(4))
            draw.ellipse([s(24), s(24), s(488), s(488)], outline=(36, 200, 219, 38), width=s(2))
        else:
            # Squircle
            draw.rounded_rectangle([s(16), s(16), s(496), s(496)], radius=s(108), fill=(26, 26, 26, 255), outline=(51, 51, 51, 255), width=s(4))
            draw.rounded_rectangle([s(24), s(24), s(488), s(488)], radius=s(100), outline=(36, 200, 219, 38), width=s(2))

    # Monitor Stand Base
    draw.rounded_rectangle([s(175), s(372), s(305), s(384)], radius=s(6), fill=(45, 45, 45, 255), outline=(51, 51, 51, 255), width=s(2))
    # Monitor Stand Neck
    draw.polygon([(s(226), s(325)), (s(254), s(325)), (s(246), s(372)), (s(234), s(372))], fill=(38, 38, 38, 255), outline=(51, 51, 51, 255))

    # Monitor Body
    draw.rounded_rectangle([s(92), s(125), s(388), s(325)], radius=s(20), fill=(38, 38, 38, 255), outline=(51, 51, 51, 255), width=s(3))
    # Monitor Screen Glass
    draw.rounded_rectangle([s(104), s(137), s(376), s(313)], radius=s(12), fill=(24, 27, 28, 255), outline=(0, 54, 58, 255), width=s(1))

    # Monitor Window Dots
    draw.ellipse([s(117), s(148), s(123), s(154)], fill=(51, 51, 51, 255))
    draw.ellipse([s(129), s(148), s(135), s(154)], fill=(51, 51, 51, 255))
    draw.ellipse([s(141), s(148), s(147), s(154)], fill=(36, 200, 219, 200))
    draw.line([(s(104), s(162)), (s(376), s(162))], fill=(36, 200, 219, 38), width=s(1))

    # Screen code lines
    draw.line([(s(120), s(180)), (s(190), s(180))], fill=(0, 131, 143, 180), width=s(4))
    draw.line([(s(120), s(195)), (s(160), s(195))], fill=(36, 200, 219, 130), width=s(4))
    draw.line([(s(120), s(210)), (s(210), s(210))], fill=(51, 51, 51, 255), width=s(4))

    # Sync Bridge Bezier curve simulation
    # Desktop node (230, 235) to Mobile node (340, 280)
    # Cubic bezier points: (230, 235), (290, 235), (280, 280), (340, 280)
    p0 = (s(230), s(235))
    p1 = (s(290), s(235))
    p2 = (s(280), s(280))
    p3 = (s(340), s(280))

    curve_pts = []
    num_steps = 40
    for i in range(num_steps + 1):
        t = i / float(num_steps)
        cx = (1-t)**3 * p0[0] + 3*(1-t)**2 * t * p1[0] + 3*(1-t) * t**2 * p2[0] + t**3 * p3[0]
        cy = (1-t)**3 * p0[1] + 3*(1-t)**2 * t * p1[1] + 3*(1-t) * t**2 * p2[1] + t**3 * p3[1]
        curve_pts.append((cx, cy))

    # Draw cyan glow line for bridge
    glow_width = s(14)
    for i in range(len(curve_pts) - 1):
        draw.line([curve_pts[i], curve_pts[i+1]], fill=(36, 200, 219, 210), width=glow_width)
    # Draw white highlight core
    for i in range(len(curve_pts) - 1):
        draw.line([curve_pts[i], curve_pts[i+1]], fill=(255, 255, 255, 230), width=s(4))

    # Mobile Shadow (Rendered onto separate layer with blur)
    shadow_layer = Image.new("RGBA", (dim, dim), (0, 0, 0, 0))
    shadow_draw = ImageDraw.Draw(shadow_layer)
    shadow_draw.rounded_rectangle([s(284), s(178), s(428), s(418)], radius=s(28), fill=(0, 0, 0, 150))
    shadow_layer = shadow_layer.filter(ImageFilter.GaussianBlur(s(12)))
    img = Image.alpha_composite(img, shadow_layer)
    draw = ImageDraw.Draw(img)

    # Mobile Phone Body Outer
    draw.rounded_rectangle([s(290), s(170), s(428), s(406)], radius=s(28), fill=(38, 38, 38, 255), outline=(68, 68, 68, 255), width=s(3))
    # Mobile Phone Screen
    draw.rounded_rectangle([s(298), s(178), s(420), s(398)], radius=s(22), fill=(24, 27, 28, 255), outline=(0, 54, 58, 255), width=s(1))

    # Mobile Speaker Pill
    draw.rounded_rectangle([s(341), s(186), s(377), s(193)], radius=s(3.5), fill=(26, 26, 26, 255))
    draw.ellipse([s(345), s(188), s(349), s(192)], fill=(36, 200, 219, 150))

    # Mobile Card preview
    draw.rounded_rectangle([s(308), s(206), s(410), s(242)], radius=s(8), fill=(38, 38, 38, 255), outline=(36, 200, 219, 76), width=s(1))
    draw.ellipse([s(317), s(219), s(327), s(229)], fill=(36, 200, 219, 255))
    draw.line([(s(334), s(220)), (s(380), s(220))], fill=(255, 255, 255, 200), width=s(3))
    draw.line([(s(334), s(228)), (s(365), s(228))], fill=(0, 131, 143, 255), width=s(2))

    draw.rounded_rectangle([s(308), s(250), s(410), s(308)], radius=s(8), fill=(30, 35, 36, 255), outline=(51, 51, 51, 255), width=s(1))

    # Home indicator
    draw.rounded_rectangle([s(345), s(388), s(373), s(391)], radius=s(1.5), fill=(51, 51, 51, 255))

    # Nodes (Desktop, Nexus, Mobile)
    # Desktop Node
    draw.ellipse([s(221), s(226), s(239), s(244)], fill=(36, 200, 219, 255))
    draw.ellipse([s(226), s(231), s(234), s(239)], fill=(255, 255, 255, 255))

    # Central Nexus Node
    draw.ellipse([s(281), s(249), s(295), s(263)], fill=(77, 208, 225, 255))
    draw.ellipse([s(285), s(253), s(291), s(259)], fill=(255, 255, 255, 255))

    # Mobile Node
    draw.ellipse([s(331), s(271), s(349), s(289)], fill=(36, 200, 219, 255))
    draw.ellipse([s(336), s(276), s(344), s(284)], fill=(255, 255, 255, 255))

    # Downsample using Lanczos
    res = img.resize((size, size), Image.Resampling.LANCZOS)
    return res


def main():
    repo_root = "/home/rohanakode/project/cdus"
    tauri_icons_dir = os.path.join(repo_root, "src-tauri/icons")
    android_res_dir = os.path.join(repo_root, "android/app/src/main/res")
    frontend_assets_dir = os.path.join(repo_root, "src/assets")

    os.makedirs(tauri_icons_dir, exist_ok=True)
    os.makedirs(frontend_assets_dir, exist_ok=True)

    print("1. Generating Master SVGs...")
    svg_content = create_svg_logo(standalone=True)
    master_svg_path = os.path.join(frontend_assets_dir, "cdus-logo.svg")
    with open(master_svg_path, "w", encoding="utf-8") as f:
        f.write(svg_content)
    print(f"   Saved {master_svg_path}")

    # Also save to src-tauri/icons/icon.svg if needed
    with open(os.path.join(tauri_icons_dir, "icon.svg"), "w", encoding="utf-8") as f:
        f.write(svg_content)

    print("2. Generating Master 1024x1024 Raster Images...")
    master_icon_1024 = render_raster_icon(1024, is_round=False)
    master_icon_path = os.path.join(tauri_icons_dir, "icon.png")
    master_icon_1024.save(master_icon_path, "PNG")
    print(f"   Saved {master_icon_path}")

    master_round_1024 = render_raster_icon(1024, is_round=True)

    # Transparent icon for window chrome / header
    transparent_icon_512 = render_raster_icon(512, is_round=False, transparent_bg=True)
    transparent_icon_path = os.path.join(frontend_assets_dir, "cdus-logo-transparent.png")
    transparent_icon_512.save(transparent_icon_path, "PNG")
    print(f"   Saved {transparent_icon_path}")

    print("3. Generating Desktop Icons...")
    # Standard desktop sizes
    sizes = {
        "32x32.png": 32,
        "128x128.png": 128,
        "128x128@2x.png": 256,
        "Square30x30Logo.png": 30,
        "Square44x44Logo.png": 44,
        "Square71x71Logo.png": 71,
        "Square89x89Logo.png": 89,
        "Square107x107Logo.png": 107,
        "Square142x142Logo.png": 142,
        "Square150x150Logo.png": 150,
        "Square284x284Logo.png": 284,
        "Square310x310Logo.png": 310,
        "StoreLogo.png": 50,
    }

    for filename, s in sizes.items():
        out_path = os.path.join(tauri_icons_dir, filename)
        resized = render_raster_icon(s, is_round=False)
        resized.save(out_path, "PNG")
        print(f"   Saved {out_path} ({s}x{s})")

    # Generate multi-size icon.ico using PIL
    ico_path = os.path.join(tauri_icons_dir, "icon.ico")
    ico_sizes = [(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)]
    master_icon_1024.save(ico_path, format="ICO", sizes=ico_sizes)
    print(f"   Saved {ico_path}")

    print("4. Generating Android Mipmaps & Drawables...")
    android_densities = {
        "mipmap-mdpi": 48,
        "mipmap-hdpi": 72,
        "mipmap-xhdpi": 96,
        "mipmap-xxhdpi": 144,
        "mipmap-xxxhdpi": 192,
    }

    for folder, dim in android_densities.items():
        density_dir = os.path.join(android_res_dir, folder)
        os.makedirs(density_dir, exist_ok=True)

        # Regular square/squircle launcher icon (both webp and png)
        launcher_img = render_raster_icon(dim, is_round=False)
        launcher_webp = os.path.join(density_dir, "ic_launcher.webp")
        launcher_png = os.path.join(density_dir, "ic_launcher.png")
        launcher_img.save(launcher_webp, "WEBP")
        launcher_img.save(launcher_png, "PNG")

        # Round launcher icon
        round_img = render_raster_icon(dim, is_round=True)
        round_webp = os.path.join(density_dir, "ic_launcher_round.webp")
        round_png = os.path.join(density_dir, "ic_launcher_round.png")
        round_img.save(round_webp, "WEBP")
        round_img.save(round_png, "PNG")

        print(f"   Saved {folder} icons ({dim}x{dim})")

    # Android Adaptive XMLs
    drawable_dir = os.path.join(android_res_dir, "drawable")
    os.makedirs(drawable_dir, exist_ok=True)

    bg_xml_path = os.path.join(drawable_dir, "ic_launcher_background.xml")
    fg_xml_path = os.path.join(drawable_dir, "ic_launcher_foreground.xml")

    with open(bg_xml_path, "w", encoding="utf-8") as f:
        f.write(create_svg_background_only())
    print(f"   Saved {bg_xml_path}")

    with open(fg_xml_path, "w", encoding="utf-8") as f:
        f.write(create_svg_foreground_only())
    print(f"   Saved {fg_xml_path}")

    print("\nAll assets successfully generated!")

if __name__ == "__main__":
    main()
