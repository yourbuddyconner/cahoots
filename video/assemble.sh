#!/bin/sh
# Assembles out/frames/*.png and out/master.wav into out/cahoots-launch.mp4 (H.264 High, yuv420p, CRF 16, AAC 192k, faststart).
set -e
cd "$(dirname "$0")"
ffmpeg -v error -y -framerate 30 -pattern_type glob -i 'out/frames/f*.png' -i out/master.wav \
  -c:v libx264 -profile:v high -pix_fmt yuv420p -crf 16 -preset slow -r 30 \
  -c:a aac -b:a 192k -ar 48000 -ac 2 -shortest -movflags +faststart out/cahoots-launch.mp4
ffprobe -v error -show_entries stream=codec_name,profile,width,height,pix_fmt,r_frame_rate,sample_rate,channels:format=duration -of default=nw=1 out/cahoots-launch.mp4
