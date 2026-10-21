#!/bin/sh
# Renders the Open Graph cards to site/assets/og-*.png with headless Chrome.
# Run from the repository root after editing site/og/*.html.
set -e
cd "$(dirname "$0")"
CHROME="${CHROME:-/Applications/Google Chrome.app/Contents/MacOS/Google Chrome}"
for name in home docs app; do
  "$CHROME" --headless=new --disable-gpu --hide-scrollbars --window-size=1200,630 \
    --virtual-time-budget=4000 --screenshot="../assets/og-$name.png" "file://$PWD/$name.html" 2>/dev/null
  echo "wrote site/assets/og-$name.png"
done
