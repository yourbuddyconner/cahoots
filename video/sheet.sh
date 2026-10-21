#!/bin/sh
# Contact sheet with frame numbers burned in: sheet.sh <frames-dir> <out.png> [cols]
set -e
dir=$1; out=$2; cols=${3:-3}
n=$(ls "$dir"/f*.png | wc -l | tr -d ' ')
rows=$(( (n + cols - 1) / cols ))
ffmpeg -v error -y -pattern_type glob -i "$dir/f*.png" -vf "drawtext=text='%{frame_num}':x=20:y=20:fontsize=48:fontcolor=white:box=1:boxcolor=black@0.6,scale=640:-1,tile=${cols}x${rows}:padding=6:color=0x222222" -frames:v 1 "$out"
