#!/bin/sh
# Angled renders of both sides, composed into hero.png. Needs the repo's video/ node_modules (Playwright + Chrome).
set -e
cd "$(dirname "$0")"
CLI=/Applications/KiCad.app/Contents/MacOS/kicad-cli
T=$(mktemp -d)
$CLI pcb render --side top --perspective --rotate "-30,0,25" --zoom 0.8 --quality high --width 2400 --height 1600 --background transparent --floor -o "$T/hero-front.png" v1.kicad_pcb >/dev/null
$CLI pcb render --side bottom --perspective --rotate "-30,0,-25" --zoom 0.8 --quality high --width 2400 --height 1600 --background transparent --floor -o "$T/hero-back.png" v1.kicad_pcb >/dev/null
cat > "$T/index.html" <<HTML
<!doctype html><meta charset="utf-8">
<style>html,body{margin:0;background:#0b0d10}.stage{position:relative;width:2400px;height:1350px;overflow:hidden;background:radial-gradient(1200px 900px at 62% 45%,#1d242b 0%,#0b0d10 70%)}img{position:absolute;filter:drop-shadow(0 40px 60px rgba(0,0,0,.6))}.front{left:560px;top:-80px;width:1900px}.back{left:-140px;top:330px;width:1250px;opacity:.95}</style>
<div class="stage"><img class="back" src="hero-back.png"><img class="front" src="hero-front.png"></div>
HTML
HERO="$(pwd)/hero.png"
(cd ../../video && node -e '
const { chromium } = require("playwright");
(async () => { const b = await chromium.launch({ channel: "chrome" }); const p = await b.newPage({ viewport: { width: 2400, height: 1350 } });
  await p.goto("file://" + process.argv[1] + "/index.html"); await p.waitForTimeout(500);
  await p.screenshot({ path: process.argv[2], clip: {x:0,y:0,width:2400,height:1350} }); await b.close(); })();' "$T" "$HERO")
echo "wrote $HERO"
