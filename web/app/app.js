// Cahoots phone/laptop client. The ceremony and pure-pad messaging make no network calls.
// The chat screen talks to a Numbers Station relay over HTTP; it sends and receives ciphertext only.
// All crypto is in cahoots-core and cahoots-chat (WASM).
import init, {
  Ceremony, Circle, FountainEncoder, FountainDecoder,
  b64_encode, b64_decode, bucket_pad, bucket_unpad, chat_bucket_pad, chat_bucket_unpad, qr_svg, qr_decode_rgba,
} from './pkg/cahoots_wasm.js';

const CLAIM = {
  TRNG: 'Confidentiality: secure against unlimited computing power, as long as pad bytes are never reused.',
  CSPRNG: 'Confidentiality: computationally secure. No hardware TRNG contributed to this pad.',
};
const ERRORS = {
  reuse: 'Possible pad reuse: this range was already spent. The message was refused and nothing was changed.',
  tag: 'Integrity check failed. The message was refused and nothing was spent.',
  'wrong-circle': 'This message belongs to a different circle.',
  'out-of-range': 'The message points outside the pad.',
  exhausted: 'Your slice of the pad is used up. Hold a new ceremony.',
  base64url: 'That is not a Cahoots message.',
  replay: 'Already received: this message counter was consumed before.',
  'too-far-ahead': 'Too many messages were skipped before this one.',
  version: 'Not a chat message.',
  length: 'Malformed message.',
};
const CHAT_LABEL = 'End-to-end encrypted, pad-hedged';
const CHAT_CLAIM = 'The pad seeds the keys and ChaCha20-Poly1305 encrypts. The relay sees ciphertext and traffic shape only.';
const DEFAULT_RELAY = 'http://127.0.0.1:8080';
const POLL_WAIT_MS = 25000;
const QR_PACKET = 600;   // payload bytes per animated frame
const QR_FPS = 10;
const TYPES = ['text', 'approve', 'vouch', 'control'];

// ---------------------------------------------------------------- storage (IndexedDB)
const db = await new Promise((resolve, reject) => {
  const req = indexedDB.open('cahoots', 2);
  req.onupgradeneeded = (ev) => {
    const d = req.result;
    if (ev.oldVersion < 1) {
      d.createObjectStore('circles', { keyPath: 'id' });
      d.createObjectStore('messages', { keyPath: 'key', autoIncrement: true }).createIndex('circle', 'circle');
    }
    if (ev.oldVersion < 2) {
      d.createObjectStore('chat', { keyPath: 'key', autoIncrement: true }).createIndex('circle', 'circle');
      d.createObjectStore('settings', { keyPath: 'id' });
    }
  };
  req.onsuccess = () => resolve(req.result);
  req.onerror = () => reject(req.error);
});
const tx = (store, mode, fn) => new Promise((resolve, reject) => {
  const t = db.transaction(store, mode);
  const r = fn(t.objectStore(store));
  t.oncomplete = () => resolve(r && r.result);
  t.onerror = () => reject(t.error);
});
const saveCircle = (circle) => tx('circles', 'readwrite', (s) => s.put({
  id: circle.circle_id_hex(), name: circle.name(), label: circle.label(), state: circle.export(), updated: Date.now(),
}));
const loadCircles = () => tx('circles', 'readonly', (s) => s.getAll());
const deleteCircle = (id) => tx('circles', 'readwrite', (s) => s.delete(id));
const saveMessage = (m) => tx('messages', 'readwrite', (s) => s.add(m));
const loadMessages = (id) => tx('messages', 'readonly', (s) => s.index('circle').getAll(id));
const saveChat = (m) => tx('chat', 'readwrite', (s) => s.add(m));
const loadChat = (id) => tx('chat', 'readonly', (s) => s.index('circle').getAll(id));
const loadSettings = async (id) => (await tx('settings', 'readonly', (s) => s.get(id))) || { id, relay: DEFAULT_RELAY };
const saveSettings = (row) => tx('settings', 'readwrite', (s) => s.put(row));

const live = new Map(); // circle id -> Circle (WASM object)
let flash = null;   // one-shot banner for the next circle render
let lastOut = null; // last sent message card, kept across re-renders
async function openCircle(id) {
  if (live.has(id)) return live.get(id);
  const rows = await loadCircles();
  const row = rows.find((r) => r.id === id);
  if (!row) return null;
  const c = Circle.import(row.state);
  live.set(id, c);
  return c;
}

// ---------------------------------------------------------------- tiny DOM helpers
const view = document.getElementById('view');
const h = (tag, attrs = {}, ...kids) => {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k === 'class') el.className = v;
    else if (k.startsWith('on')) el.addEventListener(k.slice(2), v);
    else if (k === 'html') el.innerHTML = v;
    else el.setAttribute(k, v);
  }
  for (const kid of kids.flat()) if (kid != null) el.append(kid.nodeType ? kid : document.createTextNode(kid));
  return el;
};
const render = (...nodes) => { stopScanner(); view.replaceChildren(...nodes.filter((n) => n != null)); };
window.addEventListener('hashchange', () => { chatRun++; });
const notice = (text, kind = '') => h('div', { class: `notice ${kind}` }, text);
const kib = (n) => n >= 1024 ? `${Math.round(n / 1024)} KiB` : `${n} B`;
const hexToBytes = (s) => Uint8Array.from(s.match(/../g).map((x) => parseInt(x, 16)));
const utf8 = new TextEncoder();
const utf8d = new TextDecoder();
function qrBox(text) {
  const box = h('div', { class: 'qr' });
  try { box.innerHTML = qr_svg(utf8.encode(text)); } catch (e) { box.replaceChildren(h('p', { class: 'muted' }, 'Too large for one QR code. Use the text below.')); }
  return box;
}
function copyButton(text) {
  return h('button', { onclick: async (e) => { await navigator.clipboard.writeText(text); e.target.textContent = 'Copied'; } }, 'Copy');
}
function downloadButton(name, text) {
  return h('button', { onclick: () => {
    const a = h('a', { href: URL.createObjectURL(new Blob([text], { type: 'text/plain' })), download: name });
    a.click(); URL.revokeObjectURL(a.href);
  } }, 'Download');
}

// ---------------------------------------------------------------- QR scanning (camera)
let scanner = null;
function stopScanner() {
  if (!scanner) return;
  clearInterval(scanner.timer);
  scanner.stream.getTracks().forEach((t) => t.stop());
  scanner = null;
}
/** Shows a camera preview and calls onText(text) for every code seen.
 *  Frames are decoded in our own wasm (rqrr), so this works on every browser,
 *  including iOS where WebKit has no BarcodeDetector. */
function scanBox(onText) {
  const wrap = h('div');
  if (!navigator.mediaDevices?.getUserMedia) {
    wrap.append(h('p', { class: 'muted' }, 'This browser cannot open the camera. Use paste or a file instead.'));
    return wrap;
  }
  const video = h('video', { playsinline: '', muted: '', autoplay: '' });
  const status = h('p', { class: 'muted' }, 'Starting camera…');
  const work = document.createElement('canvas');
  const ctx = work.getContext('2d', { willReadFrequently: true });
  const MAX_SIDE = 480; // decode at a reduced size: ~11 ms per frame on a laptop, plenty for a 600-byte frame
  const btn = h('button', { onclick: async () => {
    btn.disabled = true;
    try {
      const stream = await navigator.mediaDevices.getUserMedia({
        video: { facingMode: { ideal: 'environment' }, width: { ideal: 1280 }, height: { ideal: 720 } },
      });
      video.srcObject = stream; await video.play();
      let last = '', busy = false, frames = 0, hits = 0;
      const timer = setInterval(() => {
        if (busy || video.readyState < 2) return;
        busy = true;
        try {
          const scale = Math.min(1, MAX_SIDE / Math.max(video.videoWidth, video.videoHeight));
          const w = Math.max(1, Math.round(video.videoWidth * scale));
          const hgt = Math.max(1, Math.round(video.videoHeight * scale));
          if (work.width !== w || work.height !== hgt) { work.width = w; work.height = hgt; }
          ctx.drawImage(video, 0, 0, w, hgt);
          const { data } = ctx.getImageData(0, 0, w, hgt);
          frames++;
          const text = qr_decode_rgba(w, hgt, data);
          if (text && text !== last) { last = text; hits++; onText(text); }
          if (frames % 20 === 0) status.textContent = hits ? `Reading… ${hits} frames so far.` : 'Point the camera at the other screen.';
        } catch (e) { /* frame not ready */ }
        busy = false;
      }, 1000 / QR_FPS / 2);
      scanner = { stream, timer };
      status.textContent = 'Point the camera at the other screen.';
    } catch (e) { status.textContent = `Camera unavailable: ${e.message}`; btn.disabled = false; }
  } }, 'Start camera');
  wrap.append(btn, video, status);
  return wrap;
}

// ---------------------------------------------------------------- animated QR (fountain)
/** Returns an element that cycles through fountain packets of `bytes` at QR_FPS. */
function animatedQr(bytes, caption) {
  const enc = new FountainEncoder(bytes, QR_PACKET);
  const box = h('div', { class: 'qr' });
  const info = h('p', { class: 'muted' }, '');
  let i = 0;
  const tick = () => {
    box.innerHTML = qr_svg(utf8.encode(b64_encode(enc.packet(i))));
    info.textContent = `${caption}: frame ${i + 1}, ${enc.source_packets()} needed. Keep this on screen until the receiver says done.`;
    i += 1;
  };
  tick();
  const timer = setInterval(tick, 1000 / QR_FPS);
  const obs = new MutationObserver(() => { if (!document.body.contains(box)) { clearInterval(timer); obs.disconnect(); } });
  obs.observe(document.body, { childList: true, subtree: true });
  return h('div', {}, box, info);
}
/** Scanner + progress bar that resolves with the decoded bytes. */
function fountainReceiver(onDone) {
  let dec = new FountainDecoder();
  const bar = h('progress', { value: 0, max: 1 });
  const status = h('p', { class: 'muted' }, 'Waiting for frames…');
  const feed = (text) => {
    try {
      const done = dec.add(b64_decode(text));
      bar.value = dec.progress();
      status.textContent = `${Math.round(dec.progress() * 100)}% of ${kib(dec.transfer_length())}`;
      if (done) { const out = dec.result(); dec = new FountainDecoder(); status.textContent = 'Done.'; onDone(out); }
    } catch (e) { if (e !== 'magic') status.textContent = `Ignored frame: ${e}`; }
  };
  return h('div', {}, scanBox(feed), bar, status);
}

// ---------------------------------------------------------------- text/file fallback codecs
// Commitments, params, and messages fit in one QR. Reveals use the fountain or these text forms.
const enc = {
  params: (bytes) => `cahoots-params:${b64_encode(bytes)}`,
  // A commitment carries the circle parameters too, so scanning any member's commitment is enough to join.
  commit: (i, bytes, params) => `cahoots-commit:${i}:${b64_encode(bytes)}${params ? ':' + b64_encode(params) : ''}`,
  reveal: (i, bytes) => `cahoots-reveal:${i}:${b64_encode(bytes)}`,
};
function parseTagged(text) {
  const m = text.trim().match(/^cahoots-(params|commit|reveal):(?:(\d+):)?([A-Za-z0-9_-]+)(?::([A-Za-z0-9_-]+))?$/);
  if (!m) return null;
  return { kind: m[1], index: m[2] == null ? null : Number(m[2]), bytes: b64_decode(m[3]), params: m[4] ? b64_decode(m[4]) : null };
}
function paramsFromBytes(b) {
  if (b[0] !== 1) throw 'unknown version';
  return { n: b[1], pad_len: new DataView(b.buffer, b.byteOffset).getUint32(2, true), agent_seat: b[6] === 1, name: utf8d.decode(b.subarray(8, 8 + b[7])) };
}
// Commitments scanned before this device's ceremony object exists. Fed in on the run screen.
let pendingCommits = [];
// Reveal bytes carried over the fountain: [index] || contribution.
const packReveal = (i, bytes) => { const out = new Uint8Array(bytes.length + 1); out[0] = i; out.set(bytes, 1); return out; };

// ---------------------------------------------------------------- screens
async function screenHome() {
  const rows = await loadCircles();
  const list = rows.length ? rows.map((r) => h('a', { class: 'card', href: `#/c/${r.id}`, style: 'display:block;text-decoration:none;color:inherit' },
    h('div', { class: 'row' }, h('strong', { class: 'big' }, r.name), h('span', { class: `pill ${r.label.toLowerCase()}` }, r.label)),
    h('p', { class: 'muted' }, CLAIM[r.label]),
    h('p', { class: 'mono muted' }, r.id),
  )) : [h('p', { class: 'muted' }, 'No circles yet. Hold a ceremony with the people in the room.')];
  render(
    h('h1', {}, 'Your circles'),
    h('div', { class: 'row' }, h('a', { class: 'btn primary', href: '#/new' }, 'Start a ceremony'), h('a', { class: 'btn', href: '#/join' }, 'Join a ceremony')),
    ...list,
    h('p', { class: 'muted' }, 'Everything stays on this device. Pads are erased as they are used, but erasure on phone storage is best effort.'),
  );
}

function screenNew() {
  const name = h('input', { type: 'text', value: 'sf-meetup', maxlength: '64' });
  const n = h('input', { type: 'number', min: '2', max: '8', value: '2' });
  const size = h('select', {}, ...[16, 64, 256].map((k) => h('option', { value: k * 1024, ...(k === 64 ? { selected: '' } : {}) }, `${k} KiB`)));
  const agent = h('input', { type: 'checkbox' });
  render(
    h('h1', {}, 'Start a ceremony'),
    h('p', {}, 'You are the leader. Every member confirms these parameters on their own screen.'),
    h('label', {}, 'Circle name'), name,
    h('label', {}, 'Members'), n,
    h('label', {}, 'Pad length'), size,
    h('p', { class: 'muted' }, '64 KiB is a few hundred messages and takes about a minute per member over animated QR. 256 KiB is the device default.'),
    h('div', { class: 'check' }, agent, h('span', {}, h('strong', {}, 'Seat an AI agent. '), 'This circle trusts the enclave with its pad. Even Turnkey could read it. Leave this off for a human-only circle.')),
    h('p', {}),
    h('button', { class: 'primary', onclick: () => {
      const params = { n: Number(n.value), pad_len: Number(size.value), name: name.value.trim() || 'circle', agent_seat: agent.checked };
      location.hash = `#/ceremony/${b64_encode(utf8.encode(JSON.stringify(params)))}/0`;
    } }, 'Continue'),
  );
}

function screenJoin() {
  const ta = h('textarea', { placeholder: 'cahoots-commit:… or cahoots-params:…' });
  const err = h('p', { class: 'muted' });
  const go = (text) => {
    try {
      const t = parseTagged(text);
      if (!t) throw 'that is not a Cahoots code';
      if (t.kind === 'reveal') throw 'that is a reveal code. Ask for a commitment code or the parameters code';
      if (t.kind === 'commit' && !t.params) throw 'that commitment code has no parameters in it. Ask the leader to show the parameters code';
      const params = paramsFromBytes(t.kind === 'params' ? t.bytes : t.params);
      if (t.kind === 'commit') pendingCommits = [{ index: t.index, bytes: t.bytes }];
      location.hash = `#/ceremony/${b64_encode(utf8.encode(JSON.stringify(params)))}/pick`;
    } catch (e) { err.textContent = `Could not join: ${e}`; }
  };
  render(
    h('h1', {}, 'Join a ceremony'),
    h('p', {}, 'Scan any member\'s commitment code, or the leader\'s parameters code. Or paste one.'),
    scanBox(go), ta, h('p', {}), h('button', { class: 'primary', onclick: () => go(ta.value) }, 'Continue'), err,
  );
}

/** Pick index and entropy source, then create the Ceremony object. */
function screenPick(params) {
  const idx = h('select', {}, ...Array.from({ length: params.n }, (_, i) => h('option', { value: i }, `Member ${i}${i === 0 ? ' (leader)' : ''}`)));
  const src = h('select', {}, h('option', { value: 'csprng' }, 'This device only (crypto.getRandomValues) → CSPRNG label'), h('option', { value: 'file' }, 'This device + a cahoots-yk contribution file → TRNG label'), h('option', { value: 'qr' }, 'This device + animated-QR import from a laptop → TRNG label'));
  const file = h('input', { type: 'file' });
  const fileRow = h('div', { style: 'display:none' }, h('label', {}, `Contribution file (${kib(params.pad_len)} from cahoots-yk random --bytes ${params.pad_len})`), file);
  let extra = null;
  const qrRow = h('div', { style: 'display:none' });
  const qrStatus = h('p', { class: 'muted' });
  src.onchange = () => { fileRow.style.display = src.value === 'file' ? '' : 'none'; qrRow.style.display = src.value === 'qr' ? '' : 'none'; };
  qrRow.append(fountainReceiver((bytes) => { extra = bytes; qrStatus.textContent = `Imported ${kib(bytes.length)}.`; }), qrStatus);
  const err = h('p', { class: 'muted' });
  render(
    h('h1', {}, 'Confirm and prepare'),
    h('div', { class: 'card' },
      h('p', {}, h('strong', {}, params.name), ` · ${params.n} members · ${kib(params.pad_len)} pad`),
      params.agent_seat ? notice('This circle seats an AI agent. It trusts the enclave with its pad.') : h('p', { class: 'muted' }, 'Human-only circle. Even Turnkey can\'t read it.')),
    h('label', {}, 'You are'), idx,
    h('label', {}, 'Randomness'), src, fileRow, qrRow,
    h('p', { class: 'muted' }, 'A hardware source is XORed into this device\'s randomness. It never replaces it. The circle gets the TRNG label only if every byte of your contribution came from hardware.'),
    h('p', {}),
    h('button', { class: 'primary', onclick: async () => {
      try {
        if (src.value === 'file') { if (!file.files[0]) throw 'choose a file'; extra = new Uint8Array(await file.files[0].arrayBuffer()); }
        if (src.value !== 'csprng' && !extra) throw 'no hardware contribution imported yet';
        if (extra && extra.length !== params.pad_len) throw `hardware contribution must be exactly ${params.pad_len} bytes, got ${extra.length}`;
        const contribution = new Uint8Array(params.pad_len);
        for (let o = 0; o < contribution.length; o += 65536) crypto.getRandomValues(contribution.subarray(o, Math.min(o + 65536, contribution.length)));
        const c = new Ceremony(params.n, params.pad_len, params.name, params.agent_seat, Number(idx.value), contribution, extra ?? undefined);
        ceremony = { c, params, label: extra ? 'TRNG' : 'CSPRNG', peers: new Map() };
        location.hash = `#/ceremony/${b64_encode(utf8.encode(JSON.stringify(params)))}/run`;
      } catch (e) { err.textContent = `Could not start: ${e}`; }
    } }, 'Generate my contribution and commit'),
    err,
  );
}

let ceremony = null; // in-memory only: { c, params, label }

function screenRun(params) {
  if (!ceremony) { location.hash = `#/ceremony/${b64_encode(utf8.encode(JSON.stringify(params)))}/pick`; return; }
  const { c } = ceremony;
  const me = c.index();
  if (pendingCommits.length) {
    for (const p of pendingCommits) { if (p.index !== me) { try { c.add_commitment(p.index, p.bytes); } catch (e) { /* shown as waiting */ } } }
    pendingCommits = [];
  }
  const phase = c.phase();
  if (phase === 'aborted') { render(h('h1', {}, 'Ceremony aborted'), notice('A reveal did not match its commitment. Nothing was kept. Start again.', 'bad'), h('a', { class: 'btn', href: '#/' }, 'Back')); ceremony = null; return; }
  if (phase === 'complete') { screenWords(); return; }
  const others = Array.from({ length: params.n }, (_, i) => i).filter((i) => i !== me);
  const paramsText = enc.params(c.params_bytes());
  const status = h('p', { class: 'muted' });
  const refresh = () => screenRun(params);

  const feed = (text) => {
    try {
      const t = parseTagged(text);
      if (!t || t.kind === 'params') return;
      if (t.index === me) return;
      if (t.kind === 'commit') c.add_commitment(t.index, t.bytes);
      else if (t.kind === 'reveal') { if (c.phase() !== 'reveal') throw 'wrong-phase'; c.add_reveal(t.index, t.bytes); }
      refresh();
    } catch (e) { status.textContent = `Rejected: ${e}`; if (c.phase() === 'aborted') refresh(); }
  };
  const feedBytes = (bytes) => { // fountain-carried reveal: [index] || contribution
    try { if (bytes[0] === me) return; c.add_reveal(bytes[0], bytes.subarray(1)); refresh(); }
    catch (e) { status.textContent = `Rejected: ${e}`; if (c.phase() === 'aborted') refresh(); }
  };

  const nodes = [
    h('h1', {}, params.name),
    h('div', { class: 'phase' }, h('span', { class: phase === 'commit' ? 'on' : '' }, '1 Commit'), h('span', { class: phase === 'reveal' ? 'on' : '' }, '2 Reveal'), h('span', {}, '3 Check words')),
  ];
  if (phase === 'commit') {
    const mine = enc.commit(me, c.commitment(), c.params_bytes());
    nodes.push(h('div', { class: 'card' }, h('h2', {}, `Your commitment (member ${me})`), h('p', { class: 'muted' }, 'Show this to every other member. Anyone who has not joined yet can scan it to join. Nobody reveals until everyone has committed.'), qrBox(mine), h('p', { class: 'mono' }, mine), h('div', { class: 'row' }, copyButton(mine))));
    if (me === 0) nodes.push(h('details', { class: 'card' }, h('summary', {}, 'Parameters only (for pasting)'), h('p', { class: 'mono' }, paramsText), h('div', { class: 'row' }, copyButton(paramsText))));
    const missing = new Set(c.missing_commitments());
    nodes.push(h('div', { class: 'card' }, h('h2', {}, 'Commitments from others'),
      ...others.map((i) => h('div', { class: 'member' }, `Member ${i}: `, missing.has(i) ? h('span', { class: 'muted' }, 'waiting') : h('span', { class: 'ok' }, 'received'))),
      scanBox(feed),
      (() => { const ta = h('textarea', { placeholder: 'cahoots-commit:…' }); return h('div', {}, ta, h('p', {}), h('button', { onclick: () => { feed(ta.value); } }, 'Add pasted commitment')); })(),
      status));
  } else {
    const reveal = c.reveal();
    const revealText = enc.reveal(me, reveal);
    nodes.push(h('div', { class: 'card' }, h('h2', {}, `Your reveal (member ${me})`), h('p', { class: 'muted' }, 'All commitments are in. Show this animation to every other member.'),
      animatedQr(packReveal(me, reveal), 'Reveal'),
      h('details', {}, h('summary', {}, 'Text and file fallback'), h('div', { class: 'row' }, copyButton(revealText), downloadButton(`reveal-${me}.txt`, revealText)))));
    const missing = new Set(c.missing_reveals());
    const ta = h('textarea', { placeholder: 'cahoots-reveal:…' });
    const file = h('input', { type: 'file' });
    nodes.push(h('div', { class: 'card' }, h('h2', {}, 'Reveals from others'),
      ...others.map((i) => h('div', { class: 'member' }, `Member ${i}: `, missing.has(i) ? h('span', { class: 'muted' }, 'waiting') : h('span', { class: 'ok' }, 'verified against commitment'))),
      fountainReceiver(feedBytes),
      h('details', {}, h('summary', {}, 'Text and file fallback'), ta, h('p', {}), h('div', { class: 'row' }, h('button', { onclick: () => feed(ta.value) }, 'Add pasted reveal'), file, h('button', { onclick: async () => feed(await file.files[0].text()) }, 'Add file'))),
      status));
  }
  nodes.push(h('p', {}), h('button', { class: 'danger', onclick: () => { ceremony = null; location.hash = '#/'; } }, 'Abandon ceremony'));
  render(...nodes);
}

function screenWords() {
  const { c, label, params } = ceremony;
  const out = c.finish(label);
  ceremony = null;
  const words = out.words().split(' ');
  const id = out.circle_id_hex();
  render(
    h('h1', {}, 'Check words'),
    h('p', {}, 'One member reads these aloud. Everyone else confirms their screen shows the same six words in the same order.'),
    h('div', { class: 'words' }, ...words.map((w, i) => h('div', {}, w, h('small', {}, i + 1)))),
    h('p', { class: 'muted' }, `Circle ${params.name} · ${label} · id ${id.slice(0, 8)}…`),
    h('div', { class: 'row' },
      h('button', { class: 'primary', onclick: async () => { const circle = out.into_circle(); live.set(id, circle); await saveCircle(circle); location.hash = `#/c/${id}`; } }, 'They match'),
      h('button', { class: 'danger', onclick: () => { location.hash = '#/'; } }, 'They differ: discard')),
  );
}

async function screenCircle(id) {
  const circle = await openCircle(id);
  if (!circle) { render(h('p', {}, 'Unknown circle.'), h('a', { class: 'btn', href: '#/' }, 'Back')); return; }
  const label = circle.label();
  const msgs = await loadMessages(id);
  const banner = h('div', {}, flash ? notice(flash.msg, flash.kind) : null);
  flash = null;
  const text = h('textarea', { placeholder: 'Message (up to 255 bytes with padding on)' });
  const padOn = h('input', { type: 'checkbox', checked: '' });
  const type = h('select', {}, ...TYPES.map((t, i) => h('option', { value: i }, t)));
  const outBox = h('div', {}, lastOut && lastOut.id === id ? lastOut.node : null);
  const inbox = h('textarea', { placeholder: 'Paste a message (base64url)…' });

  const persist = async () => { await saveCircle(circle); };
  const show = (kind, msg) => { flash = { kind, msg }; screenCircle(id); };

  const doSend = async () => {
    try {
      let pt = utf8.encode(text.value);
      if (padOn.checked) { const p = bucket_pad(pt); if (!p) throw 'Message too long for padding (255 bytes max). Shorten it or turn padding off.'; pt = p; }
      const wire = circle.send(Number(type.value), pt);
      await persist(); // the range is spent in memory already; the wire leaves only after the ledger is saved
      await saveMessage({ circle: id, dir: 'out', type: TYPES[type.value], text: text.value, ts: Date.now() });
      lastOut = { id, node: h('div', { class: 'card' }, h('p', { class: 'muted' }, `Spent ${32 + pt.length} pad bytes. ${kib(circle.remaining())} left in your slice. Share this with the circle.`), qrBox(wire), h('p', { class: 'mono' }, wire), h('div', { class: 'row' }, copyButton(wire))) };
      await screenCircle(id);
    } catch (e) { show('bad', ERRORS[e] || String(e)); }
  };
  const doReceive = async (wire) => {
    try {
      const r = circle.receive(wire.trim());
      await persist();
      let pt = r.plaintext();
      const un = bucket_unpad(pt);
      const shown = utf8d.decode(un ?? pt);
      await saveMessage({ circle: id, dir: 'in', type: TYPES[r.msg_type()], text: shown, slice: r.slice(), ts: Date.now() });
      show('ok', `Decrypted a ${TYPES[r.msg_type()]} message from slice ${r.slice()}.`);
    } catch (e) { show('bad', ERRORS[e] || `Refused: ${e}`); }
  };

  render(
    h('div', { class: 'row' }, h('h1', {}, circle.name()), h('span', { class: `pill ${label.toLowerCase()}` }, label)),
    h('p', { class: 'muted' }, CLAIM[label]),
    h('p', { class: 'muted' }, `Integrity inside the circle is information-theoretic (one-time Poly1305). The tag proves a member wrote it, not which one. You are member ${circle.my_slice()} of ${circle.members()}${circle.agent_seat() ? ', plus an agent seat' : ''}. ${kib(circle.remaining())} of ${kib(circle.slice_len())} left in your slice.`),
    circle.remaining() < circle.slice_len() / 10 ? notice('Less than 10% of your slice remains. Plan a new ceremony.') : null,
    banner,
    h('div', { class: 'row' }, h('a', { class: 'btn primary', href: `#/chat/${id}` }, 'Open chat'), h('span', { class: 'muted' }, 'Pad-seeded chat over a relay. Below: pure-pad messages, one byte of pad per byte.')),
    h('h2', {}, 'Compose'),
    type, text,
    h('div', { class: 'check' }, padOn, h('span', {}, 'Pad to 32/64/128/256 bytes to hide the length')),
    h('button', { class: 'primary', onclick: doSend }, 'Encrypt and spend'),
    outBox,
    h('h2', {}, 'Read'),
    scanBox((t) => { if (!t.startsWith('cahoots-')) doReceive(t); }),
    inbox, h('p', {}), h('button', { onclick: () => doReceive(inbox.value) }, 'Decrypt'),
    h('h2', {}, 'History'),
    ...(msgs.length ? msgs.reverse().map((m) => h('div', { class: 'msg' }, h('div', { class: 'meta' }, `${m.dir === 'out' ? 'sent' : `from slice ${m.slice}`} · ${m.type} · ${new Date(m.ts).toLocaleString()}`), h('div', {}, m.text))) : [h('p', { class: 'muted' }, 'No messages yet.')]),
    h('p', { class: 'muted' }, 'Plaintext history is stored on this device so you can reread it. Pad bytes are erased as they are used. Erasure on phone storage is best effort.'),
    h('p', {}),
    h('button', { class: 'danger', onclick: async () => { live.delete(id); await deleteCircle(id); location.hash = '#/'; } }, 'Delete circle from this device'),
  );
}

// ---------------------------------------------------------------- chat (pad-seeded, over the Numbers Station)
let chatRun = 0; // bumps on every render; a stale poll loop sees a different value and stops
async function screenChat(id) {
  const circle = await openCircle(id);
  if (!circle) { render(h('p', {}, 'Unknown circle.'), h('a', { class: 'btn', href: '#/' }, 'Back')); return; }
  const me = circle.my_slice();
  const settings = await loadSettings(id);
  const run = ++chatRun;
  const persist = () => saveCircle(circle);

  const relayIn = h('input', { type: 'text', value: settings.relay, placeholder: DEFAULT_RELAY });
  const relayBase = () => (relayIn.value.trim() || location.origin).replace(/\/$/, '');
  const status = h('p', { class: 'muted' }, '');
  const epochChip = h('span', { class: 'pill' }, '');
  const list = h('div', { class: 'chatlist' });
  const input = h('input', { type: 'text', placeholder: 'Message (up to 4094 bytes, padded to 64/256/1024/4096)', maxlength: '4000' });
  const sendBtn = h('button', { class: 'primary' }, 'Send');
  const rotateBtn = h('button', {}, 'Rotate epoch');

  const setEpoch = () => { const e = circle.current_epoch(); epochChip.textContent = e ? `epoch ${e}` : 'no epoch yet'; };
  const line = (m) => h('div', { class: `chatmsg ${m.dir}` },
    h('div', { class: 'meta' }, m.dir === 'rejected' ? `rejected: ${m.reason} · seq ${m.seq}` : `${m.dir === 'out' ? 'you' : `member ${m.sender}`} · `, m.dir === 'rejected' ? null : h('span', { class: 'pill' }, `epoch ${m.epoch}`), m.dir === 'rejected' ? null : ` · ${new Date(m.ts).toLocaleTimeString()}`),
    m.dir === 'rejected' ? null : h('div', {}, m.text));
  const append = (m) => { list.append(line(m)); list.scrollTop = list.scrollHeight; };
  for (const m of await loadChat(id)) append(m);
  setEpoch();

  const post = async (bytes) => {
    const res = await fetch(`${relayBase()}/v1/circles/${id}/messages`, { method: 'POST', headers: { 'Content-Type': 'application/octet-stream' }, body: bytes });
    if (!res.ok) throw `relay ${res.status}: ${await res.text()}`;
    return res.json();
  };
  const doSend = async () => {
    const text = input.value;
    if (!text) return;
    sendBtn.disabled = true;
    try {
      const pt = chat_bucket_pad(utf8.encode(text));
      if (!pt) throw 'Message too long.';
      const wire = circle.chat_send(pt);
      await persist(); // keys and any epoch spend are recorded before the ciphertext leaves
      await post(b64_decode(wire));
      const m = { circle: id, dir: 'out', sender: me, epoch: circle.current_epoch(), text, ts: Date.now() };
      await saveChat(m); append(m); setEpoch();
      input.value = '';
    } catch (e) { status.textContent = `Send failed: ${ERRORS[e] || e}`; }
    sendBtn.disabled = false;
  };
  const doRotate = async () => {
    try { const e = circle.rotate_epoch(); await persist(); setEpoch(); status.textContent = `Started epoch ${e}: spent 32 pad bytes. Peers switch when your next message arrives.`; }
    catch (e) { status.textContent = `Rotate failed: ${ERRORS[e] || e}`; }
  };
  sendBtn.onclick = doSend;
  input.onkeydown = (e) => { if (e.key === 'Enter') doSend(); };
  rotateBtn.onclick = doRotate;

  // Long-poll loop. Stops when the screen is replaced.
  const poll = async () => {
    while (run === chatRun) {
      const after = circle.relay_seq();
      const url = `${relayBase()}/v1/circles/${id}/messages?wait=${POLL_WAIT_MS}${after == null ? '' : `&after=${after}`}`;
      let data;
      try {
        const res = await fetch(url);
        if (!res.ok) throw `${res.status}`;
        data = await res.json();
        status.textContent = `Connected to relay. Last seq ${circle.relay_seq() ?? 'none'}.`;
      } catch (e) {
        status.textContent = `Relay unreachable (${e}). Retrying…`;
        await new Promise((r) => setTimeout(r, 3000));
        continue;
      }
      if (run !== chatRun) break;
      let changed = false;
      for (const entry of data.entries || []) {
        const bytes = b64_decode(entry.message);
        if (bytes[0] === 2) {
          const sender = bytes[22];
          if (sender !== me) {
            try {
              const r = circle.chat_receive(entry.message);
              const un = chat_bucket_unpad(r.text());
              const m = { circle: id, dir: 'in', sender: r.sender(), epoch: r.epoch(), text: utf8d.decode(un ?? r.text()), ts: entry.received_at, seq: entry.seq };
              await saveChat(m); append(m);
            } catch (e) {
              const m = { circle: id, dir: 'rejected', reason: e, seq: entry.seq, ts: entry.received_at };
              await saveChat(m); append(m);
            }
          }
        }
        circle.set_relay_seq(BigInt(entry.seq));
        changed = true;
      }
      if (changed) { await persist(); setEpoch(); }
    }
  };

  render(
    h('div', { class: 'row' }, h('h1', {}, circle.name()), epochChip),
    h('p', {}, h('strong', {}, CHAT_LABEL), '. ', CHAT_CLAIM),
    h('p', { class: 'muted' }, `You are member ${me}. One epoch costs 32 pad bytes for the whole circle. ${kib(circle.remaining())} left in your slice. The sender shown is a chain label, not a proof of who typed.`),
    h('details', {}, h('summary', {}, 'Relay'), h('label', {}, 'Numbers Station URL (blank = this origin)'), relayIn, h('p', {}), h('button', { onclick: async () => { await saveSettings({ id, relay: relayIn.value.trim() }); chatRun++; screenChat(id); } }, 'Save and reconnect')),
    status,
    list,
    h('div', { class: 'row' }, input, sendBtn, rotateBtn),
    h('p', { class: 'muted' }, 'Plaintext history is stored on this device. Erasure on phone storage is best effort.'),
    h('a', { class: 'btn', href: `#/c/${id}` }, 'Back to circle'),
  );
  poll();
}

// ---------------------------------------------------------------- router
async function route() {
  const parts = location.hash.replace(/^#\/?/, '').split('/');
  try {
    if (parts[0] === 'new') screenNew();
    else if (parts[0] === 'join') screenJoin();
    else if (parts[0] === 'ceremony') {
      const params = JSON.parse(utf8d.decode(b64_decode(parts[1])));
      if (parts[2] === 'run') screenRun(params); else screenPick(params);
    } else if (parts[0] === 'c') await screenCircle(parts[1]);
    else if (parts[0] === 'chat') await screenChat(parts[1]);
    else await screenHome();
  } catch (e) { render(h('p', {}, `Error: ${e}`), h('a', { class: 'btn', href: '#/' }, 'Home')); }
}

await init();
window.addEventListener('hashchange', route);
if ('serviceWorker' in navigator && location.protocol !== 'file:') navigator.serviceWorker.register('./sw.js').catch(() => {});
route();
