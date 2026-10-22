// Timeline audit: holds, slot clashes, chapter bounds, dead air. Run: npx tsx audit.mts
import { DEAD_AIR, CH, HITS, MOVES, TEXTS, goneAt } from "./src/timeline";
import { DUR, FPS, STAGGER, minHold } from "./src/motion";

let fails = 0;
const fail = (msg: string) => { fails++; console.log("FAIL", msg); };
const sec = (f: number) => (f / FPS).toFixed(2);

// 1. Every text holds at least 0.3 s per word + 0.5 s once fully revealed. A text with a pause
//    is two sentences, each judged from the frame its own last word lands.
for (const t of TEXTS) {
  const all = t.text.trim().split(/\s+/);
  const segments = t.pause ? [[all.slice(0, t.pause.after), 0], [all.slice(t.pause.after), t.pause.frames]] as const : [[all, 0]] as const;
  let first = 0;
  for (const [words, delay] of segments) {
    const shown = t.in + delay + (words.length - 1) * STAGGER.word + DUR.textIn;
    const hold = t.out - shown;
    const need = minHold(words.join(" "));
    const ok = hold >= need;
    console.log(`${ok ? "ok  " : "FAIL"} ${t.id.padEnd(16)} ${sec(t.in + delay)}-${sec(t.out)}  hold ${sec(hold)} s (min ${sec(need)}) ${words.length}w slot=${t.slot ?? "-"}`);
    if (!ok) fails++;
    first += words.length;
  }
}
// 2. A text in a slot starts only after the previous one in that slot has fully left.
const bySlot = new Map<string, typeof TEXTS>();
for (const t of TEXTS) if (t.slot) bySlot.set(t.slot, [...(bySlot.get(t.slot) ?? []), t]);
for (const [slot, list] of bySlot) {
  list.sort((a, b) => a.in - b.in);
  for (let i = 1; i < list.length; i++) {
    const prev = list[i - 1], cur = list[i];
    const gone = goneAt(prev.text, prev.out);
    if (cur.in < gone) fail(`slot ${slot}: "${cur.id}" in at f${cur.in} before "${prev.id}" has gone (f${gone})`);
  }
}
// 3. Dead air: no chapter goes longer than DEAD_AIR without a new move, hit, or text.
const events = [...MOVES.map((m) => m.from), ...HITS.map((h) => h.f), ...TEXTS.map((t) => t.in)].sort((a, b) => a - b);
for (const [name, [a, b]] of Object.entries(CH)) {
  let last = a;
  for (const e of events) {
    if (e < a || e >= b) continue;
    if ((e - last) / FPS > DEAD_AIR) fail(`dead air ${sec(e - last)} s in ${name} before f${e}`);
    last = e;
  }
  if ((b - last) / FPS > DEAD_AIR) fail(`dead air ${sec(b - last)} s at the end of ${name}`);
}
// 4. Hits one frame before a beat, and no two hits on one frame.
const seen = new Set<number>();
for (const h of HITS) {
  if (seen.has(h.f)) fail(`two hits on f${h.f} (${h.note})`);
  seen.add(h.f);
}
console.log(`${TEXTS.length} texts, ${MOVES.length} moves, ${HITS.length} hits; ${fails} failed`);
process.exit(fails ? 1 : 0);
