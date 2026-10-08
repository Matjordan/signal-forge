#!/usr/bin/env python3
"""Generate Linux RGBA icons from the canonical artwork; requires ImageMagick."""
from pathlib import Path
import subprocess

root = Path(__file__).resolve().parents[1]
for size in (16, 24, 32, 48, 64, 128, 256, 512):
    target = root / 'assets/icons' / f'signal-forge-{size}.png'
    subprocess.run(['convert', str(root / 'assets/signal-forge-source.png'),
                    '-resize', f'{size}x{size}', '-strip', f'PNG32:{target}'], check=True)
