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
<link href="https://fonts.googleapis.com/css2?family=Inter+Tight:wght@600&family=JetBrains+Mono:wght@500&display=swap" rel="stylesheet">
<style>.brand{position:absolute;left:96px;top:84px;display:flex;align-items:center;gap:34px;color:#E8E4DA;font:600 118px/1 "Inter Tight",system-ui,sans-serif;letter-spacing:-0.035em;text-shadow:0 6px 30px rgba(0,0,0,.6)}.brand svg{width:132px;height:132px;filter:drop-shadow(0 6px 24px rgba(0,0,0,.6))}.sub{position:absolute;left:262px;top:230px;color:#C8783C;font:500 44px/1 "JetBrains Mono",ui-monospace,monospace;letter-spacing:0.08em;text-transform:uppercase;text-shadow:0 4px 20px rgba(0,0,0,.6)}</style>
<div class="stage"><img class="back" src="hero-back.png"><img class="front" src="hero-front.png">
<div class="brand"><svg viewBox="0 0 64 64"><rect width="64" height="64" rx="12" fill="#1B2228"/><circle cx="32" cy="32" r="22" fill="none" stroke="#C8783C" stroke-width="3"/><circle cx="32" cy="32" r="13" fill="none" stroke="#E8E4DA" stroke-width="3"/><circle cx="32" cy="32" r="4" fill="#F2B544"/></svg><span>Cahoots</span></div><div class="sub">Badge · V1</div></div>
HTML
HERO="$(pwd)/hero.png"
(cd ../../video && node -e '
const { chromium } = require("playwright");
(async () => { const b = await chromium.launch({ channel: "chrome" }); const p = await b.newPage({ viewport: { width: 2400, height: 1350 } });
  await p.goto("file://" + process.argv[1] + "/index.html"); await p.waitForTimeout(1500); await p.evaluate(() => document.fonts.ready);
  await p.screenshot({ path: process.argv[2], clip: {x:0,y:0,width:2400,height:1350} }); await b.close(); })();' "$T" "$HERO")
echo "wrote $HERO"
