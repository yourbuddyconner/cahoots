// Cahoots phone/laptop client. The ceremony and pure-pad messaging make no network calls.
// The chat screen talks to a Numbers Station relay over HTTP; it sends and receives ciphertext only.
// All crypto is in cahoots-core and cahoots-chat (WASM).
//
// UI rules: one job per screen. A top bar, one stage element, one primary action. Every screen says
// what to do next in one line. Nothing the person needs is behind a scroll.
import init, {
  Ceremony, Circle, FountainEncoder, FountainDecoder,
  b64_encode, b64_decode, bucket_pad, bucket_unpad, chat_bucket_pad, chat_bucket_unpad, qr_svg, qr_decode_rgba,
} from './pkg/cahoots_wasm.js';

const CLAIM = {
  TRNG: 'Secure against unlimited computing power, as long as no pad byte is reused.',
  CSPRNG: 'Computationally secure. No hardware randomness went into this pad.',
};
const ERRORS = {
  reuse: 'Possible pad reuse: this range was already spent. Refused, nothing changed.',
  tag: 'Integrity check failed. Refused, nothing spent.',
  'wrong-circle': 'This message belongs to a different cahoot.',
  'out-of-range': 'The message points outside the pad.',
  exhausted: 'Your share of the pad is used up. Hold a new ceremony.',
  base64url: 'That is not a Cahoots message.',
  replay: 'Already received.',
  'too-far-ahead': 'Too many messages were skipped before this one.',
  version: 'Not a chat message.',
  length: 'Malformed message.',
  'wrong-phase': 'Not yet. Every commitment has to be in first.',
  aborted: 'This ceremony was stopped.',
};
const friendly = (e) => {
  const s = String(e);
  if (ERRORS[s]) return ERRORS[s];
  const m = s.match(/^(conflicting-commit|commit-mismatch|bad-index):(\d+)$/);
  if (m && m[1] === 'conflicting-commit') return `Two different people are using seat ${Number(m[2]) + 1}. One of them needs to leave and rejoin with another seat.`;
  if (m && m[1] === 'commit-mismatch') return `Seat ${Number(m[2]) + 1}'s reveal does not match their commitment.`;
  if (m && m[1] === 'bad-index') return 'That code is for a different group size.';
  const m2 = s.match(/^(corrupted|bad-key):(\d+)$/);
  if (m2 && m2[1] === 'corrupted') return 'That scan came through garbled. Scan it again.';
  if (m2 && m2[1] === 'bad-key') return `Seat ${Number(m2[2]) + 1}'s code is not valid. Ask them to show it again.`;
  return s;
};
const CHAT_LABEL = 'End-to-end encrypted, pad-hedged';
const CHAT_CLAIM = 'The pad seeds the keys and ChaCha20-Poly1305 encrypts. The relay sees ciphertext and traffic shape only.';
const DEFAULT_RELAY = '';
const POLL_WAIT_MS = 25000;
const QR_PACKET = 300;   // payload bytes per animated frame: ~77 modules per side, readable by phone cameras
const QR_FPS = 10;
const TYPES = ['text', 'approve', 'vouch', 'control'];
const SITE = 'https://cahoots.network';
/** Seconds a phone needs to scan a reveal of this size, allowing for missed frames. */
const scanSeconds = (padLen) => Math.round((Math.ceil(padLen / QR_PACKET) / QR_FPS) * 1.4);
const SIZES = [
  [16384, 'Small', '16 KiB · about 100 messages · a few seconds to scan'],
  [65536, 'Medium', '64 KiB · a few hundred messages · about half a minute to scan'],
  [262144, 'Large', '256 KiB · a couple thousand messages · about two minutes to scan'],
];

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
  remaining: circle.remaining(), slice_len: circle.slice_len(), members: circle.members(),
}));
const loadCircles = () => tx('circles', 'readonly', (s) => s.getAll());
const deleteCircle = (id) => tx('circles', 'readwrite', (s) => s.delete(id));
const saveMessage = (m) => tx('messages', 'readwrite', (s) => s.add(m));
const loadMessages = (id) => tx('messages', 'readonly', (s) => s.index('circle').getAll(id));
const saveChat = (m) => tx('chat', 'readwrite', (s) => s.add(m));
const loadChat = (id) => tx('chat', 'readonly', (s) => s.index('circle').getAll(id));
const loadSettings = async (id) => (await tx('settings', 'readonly', (s) => s.get(id))) || { id, relay: DEFAULT_RELAY };
const saveSettings = (row) => tx('settings', 'readwrite', (s) => s.put(row));
const nameOf = (settings, seat) => settings.names?.[seat] || `Seat ${seat + 1}`;

const live = new Map(); // circle id -> Circle (WASM object)
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
const app = document.getElementById('app');
const h = (tag, attrs = {}, ...kids) => {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (v == null || v === false) continue;
    if (k === 'class') el.className = v;
    else if (k.startsWith('on')) el.addEventListener(k.slice(2), v);
    else if (k === 'html') el.innerHTML = v;
    else el.setAttribute(k, v === true ? '' : v);
  }
  for (const kid of kids.flat()) if (kid != null && kid !== false) el.append(kid.nodeType ? kid : document.createTextNode(kid));
  return el;
};
const utf8 = new TextEncoder();
const utf8d = new TextDecoder();
const kib = (n) => n >= 1024 ? `${Math.round(n / 1024)} KiB` : `${n} B`;
const go = (hash) => { location.hash = hash; };
const buzz = (ms = 30) => { try { navigator.vibrate?.(ms); } catch { /* not supported */ } };

/** A short confirmation at the top of the screen. */
let toastTimer = null;
function toast(text, kind = '') {
  let el = document.getElementById('toast');
  if (!el) { el = h('div', { id: 'toast', role: 'status' }); document.body.append(el); }
  el.textContent = text;
  el.className = `toast ${kind} on`;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => el.classList.remove('on'), 2200);
}

/** The screen skeleton: bar (back, title, menu), body with a stage, pinned actions. */
function screen({ title, sub, back, menu, body, actions = [] }) {
  stopScanner();
  const bar = h('div', { class: 'bar' },
    back ? h('button', { 'aria-label': 'Back', onclick: () => (typeof back === 'function' ? back() : go(back)) }, '‹') : h('span'),
    h('div', { class: 'title' }, title, sub ? h('small', {}, sub) : null),
    menu ? h('button', { 'aria-label': 'More options', onclick: menu }, '⋯') : h('span'));
  const stage = h('div', { class: 'stage' }, ...body);
  const el = h('div', { class: 'screen' }, bar, h('div', { class: 'body' }, stage), actions.length ? h('div', { class: 'actions' }, ...actions) : null);
  app.replaceChildren(el);
  window.scrollTo(0, 0);
  return el;
}
const btn = (label, onclick, kind = '') => h('button', { class: `btn ${kind}`, onclick }, label);
const link = (label, onclick) => h('button', { class: 'link', onclick }, label);
const notice = (text, kind = '') => h('div', { class: `notice ${kind}` }, text);
const status = () => h('p', { class: 'status', role: 'status' }, '');
const next = (text) => h('p', { class: 'next' }, text); // the one line that says what to do now
function seg(options, value, onChange) {
  const el = h('div', { class: 'seg', role: 'group' });
  const set = (v) => { value = v; for (const b of el.children) b.classList.toggle('on', b.dataset.v === String(v)); onChange(v); };
  for (const [v, label, disabled] of options) el.append(h('button', { 'data-v': v, type: 'button', class: String(v) === String(value) ? 'on' : '', disabled: !!disabled, onclick: () => set(v) }, label));
  return el;
}
function field(label, input, hint) { return h('div', { class: 'field' }, h('label', {}, label), input, hint ? h('span', { class: 'hint' }, hint) : null); }
const input = (attrs) => h('input', { class: 'input', ...attrs });
function stepper(min, max, value, onChange) {
  const out = h('output', {}, String(value));
  const set = (v) => { value = Math.min(max, Math.max(min, v)); out.textContent = String(value); onChange(value); };
  return h('div', { class: 'stepper' }, h('button', { type: 'button', onclick: () => set(value - 1), 'aria-label': 'Fewer' }, '−'), out, h('button', { type: 'button', onclick: () => set(value + 1), 'aria-label': 'More' }, '+'));
}
/** A bottom sheet for the secondary options a screen needs but shouldn't show by default. */
function sheet(...kids) {
  const bg = h('div', { class: 'sheet-bg', onclick: (e) => { if (e.target === bg) close(); } });
  const close = () => { bg.classList.remove('on'); setTimeout(() => bg.remove(), 180); };
  const onKey = (e) => { if (e.key === 'Escape') { close(); document.removeEventListener('keydown', onKey); } };
  document.addEventListener('keydown', onKey);
  bg.append(h('div', { class: 'sheet', role: 'dialog' }, h('div', { class: 'grab' }), ...kids.map((k) => (typeof k === 'function' ? k(close) : k))));
  document.body.append(bg);
  requestAnimationFrame(() => bg.classList.add('on'));
  return close;
}
function panelQr(text, capLeft, capRight) {
  const box = h('div', { class: 'qr' });
  try { box.innerHTML = qr_svg(utf8.encode(text)); } catch (e) { box.replaceChildren(h('p', { class: 'muted' }, 'Too large for one code. Use the text form in the menu.')); }
  return h('div', { class: 'panel' }, h('div', { class: 'cap' }, h('span', {}, capLeft), h('span', {}, capRight)), box);
}
const copyBtn = (text, label = 'Copy as text') => btn(label, async () => { await navigator.clipboard.writeText(text); toast('Copied'); });
const downloadBtn = (name, text) => btn('Download as a file', () => {
  const a = h('a', { href: URL.createObjectURL(new Blob([text], { type: 'text/plain' })), download: name });
  a.click(); URL.revokeObjectURL(a.href);
});
function pasteSheet(placeholder, onText, title = 'Paste a code') {
  return sheet((close) => {
    const ta = h('textarea', { class: 'input', placeholder, autofocus: true });
    return h('div', { class: 'stack' }, h('h2', {}, title), ta, btn('Use this', () => { onText(ta.value); close(); }, 'primary'));
  });
}
function fileBtn(label, onText) {
  const f = h('input', { type: 'file', style: 'display:none', onchange: async (e) => { if (e.target.files[0]) onText(await e.target.files[0].text()); e.target.value = ''; } });
  return h('div', {}, f, btn(label, () => f.click()));
}
const aboutSheet = () => sheet(
  h('h2', {}, 'What this is'),
  h('p', { class: 'muted' }, 'Cahoots makes a shared secret with the people in the room. Phones show and scan codes; no server is involved. The secret is a one-time pad: short messages encrypted with it cannot be broken by any computer, and it also seeds keys for chat.'),
  h('p', { class: 'muted' }, 'You need at least two devices in the same place. Everything stays on this device.'),
  h('a', { class: 'btn', href: SITE, target: '_blank', rel: 'noopener' }, 'Read more at cahoots.network'));

// ---------------------------------------------------------------- QR scanning (camera)
let scanner = null;
let cameraOk = sessionStorage.getItem('cam') === '1';
function stopScanner() {
  if (!scanner) return;
  clearInterval(scanner.timer);
  scanner.stream.getTracks().forEach((t) => t.stop());
  scanner = null;
}
/** A camera stage. Decodes frames in our own wasm (rqrr), so it works on every browser, iOS included. */
function camera(onText, hint = 'Fill the frame with the code') {
  const box = h('div', { class: 'cam' });
  if (!navigator.mediaDevices?.getUserMedia) {
    box.append(h('div', { class: 'cam-msg' }, h('p', {}, 'This browser cannot open the camera.'), h('p', { class: 'muted' }, 'Use "Paste a code" below instead.')));
    return box;
  }
  const video = h('video', { playsinline: true, muted: true, autoplay: true });
  const hintEl = h('div', { class: 'hint' }, hint);
  const work = document.createElement('canvas');
  const ctx = work.getContext('2d', { willReadFrequently: true });
  const MAX_SIDE = 720;
  const start = async () => {
    box.replaceChildren(video, h('div', { class: 'frame' }), hintEl);
    try {
      const stream = await navigator.mediaDevices.getUserMedia({
        video: { facingMode: { ideal: 'environment' }, width: { ideal: 1280 }, height: { ideal: 720 } },
      });
      video.srcObject = stream; await video.play();
      cameraOk = true; sessionStorage.setItem('cam', '1');
      let last = '', busy = false;
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
          const text = qr_decode_rgba(w, hgt, data);
          if (text && text !== last) { last = text; onText(text); }
        } catch (e) { /* frame not ready */ }
        busy = false;
      }, 1000 / QR_FPS / 2);
      scanner = { stream, timer, hint: hintEl };
    } catch (e) {
      const denied = /denied|permission/i.test(e.message);
      box.replaceChildren(h('div', { class: 'cam-msg' },
        h('p', {}, denied ? 'Camera access was turned off for this site.' : 'The camera could not start.'),
        h('p', { class: 'muted' }, denied ? 'Allow the camera in your browser settings, or paste a code instead.' : e.message),
        btn('Try again', start)));
    }
  };
  if (cameraOk) start(); else box.append(h('div', { class: 'cam-msg' }, h('p', { class: 'muted' }, 'The camera only reads codes. Nothing is recorded.'), btn('Open the camera', start, 'primary')));
  return box;
}
const setHint = (text) => { if (scanner?.hint) scanner.hint.textContent = text; };

// ---------------------------------------------------------------- animated QR (fountain)
function animatedQr(bytes, capLeft) {
  const enc = new FountainEncoder(bytes, QR_PACKET);
  const box = h('div', { class: 'qr' });
  const cap = h('span', {}, '');
  let i = 0;
  const tick = () => {
    box.innerHTML = qr_svg(utf8.encode(b64_encode(enc.packet(i))));
    cap.textContent = `looping · ${enc.source_packets()} frames`;
    i += 1;
  };
  tick();
  const timer = setInterval(tick, 1000 / QR_FPS);
  const obs = new MutationObserver(() => { if (!document.body.contains(box)) { clearInterval(timer); obs.disconnect(); } });
  obs.observe(document.body, { childList: true, subtree: true });
  return h('div', { class: 'panel' }, h('div', { class: 'cap' }, h('span', {}, capLeft), cap), box);
}
/** Camera plus progress; calls onDone(bytes) when an animated transfer completes. Single codes
 * (cahoots-… text) go to onTagged instead, so one camera reads both kinds. */
function fountainReceiver(onDone, hint, onTagged = null) {
  let dec = new FountainDecoder();
  const bar = h('progress', { value: 0, max: 1 });
  const st = status();
  st.textContent = onTagged ? '' : 'Hold steady on the animation. The bar fills as frames arrive.';
  const feed = (text) => {
    if (onTagged && text.startsWith('cahoots-')) { onTagged(text); return; }
    try {
      const done = dec.add(b64_decode(text));
      bar.value = dec.progress();
      st.textContent = `Reading an animation · ${Math.round(dec.progress() * 100)}% of ${kib(dec.transfer_length())}. Hold steady.`;
      if (done) { const out = dec.result(); dec = new FountainDecoder(); bar.value = 0; st.textContent = 'Got it.'; onDone(out); }
    } catch (e) { if (e !== 'magic') st.textContent = friendly(e); }
  };
  return { el: h('div', { class: 'stack' }, camera(feed, hint), bar, st), feed };
}

// ---------------------------------------------------------------- text/file codecs
// Text forms, for paste and files. A commitment is 32 bytes of hash plus the member's 1216-byte
// KEM public key; it carries the circle parameters too, so scanning any member is enough to join.
// A reveal is a sealed blob, encrypted to every other member (crates/core/src/reveal.rs).
const enc = {
  params: (bytes) => `cahoots-params:${b64_encode(bytes)}`,
  commit: (i, bytes, params) => `cahoots-commit:${i}:${b64_encode(bytes)}${params ? ':' + b64_encode(params) : ''}`,
  reveal: (i, bytes) => `cahoots-reveal:${i}:${b64_encode(bytes)}`,
};
// Binary forms, for the animated code: a type byte, then the same content.
const PACK = { commit: 1, reveal: 2 };
function packCommit(i, params, bytes) {
  const out = new Uint8Array(3 + params.length + bytes.length);
  out[0] = PACK.commit; out[1] = i; out[2] = params.length;
  out.set(params, 3); out.set(bytes, 3 + params.length);
  return out;
}
function packReveal(bytes) { const out = new Uint8Array(bytes.length + 1); out[0] = PACK.reveal; out.set(bytes, 1); return out; }
/** Decode an animated transfer into the same shape parseTagged gives. */
function parsePacked(bytes) {
  if (bytes[0] === PACK.commit) { const plen = bytes[2]; return { kind: 'commit', index: bytes[1], params: bytes.subarray(3, 3 + plen), bytes: bytes.subarray(3 + plen) }; }
  if (bytes[0] === PACK.reveal) { return { kind: 'reveal', index: bytes[2], params: null, bytes: bytes.subarray(1) }; }
  return null;
}
/** Our sealed reveal. Built once by the core; the randomness is only drawn the first time. */
const revealOf = (c) => c.reveal(crypto.getRandomValues(new Uint8Array(c.reveal_randomness_len())));
function parseTagged(text) {
  const m = text.trim().match(/^cahoots-(params|commit|reveal):(?:(\d+):)?([A-Za-z0-9_-]+)(?::([A-Za-z0-9_-]+))?$/);
  if (!m) return null;
  return { kind: m[1], index: m[2] == null ? null : Number(m[2]), bytes: b64_decode(m[3]), params: m[4] ? b64_decode(m[4]) : null };
}
function paramsFromBytes(b) {
  if (b[0] !== 1) throw 'unknown version';
  return { n: b[1], pad_len: new DataView(b.buffer, b.byteOffset).getUint32(2, true), agent_seat: b[6] === 1, name: utf8d.decode(b.subarray(8, 8 + b[7])) };
}
const paramsHash = (params) => b64_encode(utf8.encode(JSON.stringify(params)));
let pendingCommits = [];   // commitments scanned before this device's ceremony object exists
let ceremony = null;       // in-memory only: { c, params, label, mode }

// ================================================================ screens
async function screenHome() {
  const rows = (await loadCircles()).sort((a, b) => (b.updated || 0) - (a.updated || 0));
  const row = (r) => {
    const left = r.slice_len ? Math.max(0, Math.round((r.remaining / r.slice_len) * 100)) : null;
    return h('a', { class: 'row-item', href: `#/c/${r.id}` },
      h('div', {}, h('b', {}, r.name), h('span', {}, `${r.members ? `${r.members} people · ` : ''}${left == null ? '' : `${left}% of your share left · `}`), h('span', { class: `pill ${r.label.toLowerCase()}` }, r.label)),
      h('span', { class: 'arrow' }, '›'));
  };
  const empty = h('div', { class: 'empty' },
    h('img', { src: 'icon.svg', alt: '', width: 56, height: 56 }),
    h('h1', {}, 'Make a secret with the people in the room.'),
    h('p', { class: 'lead' }, 'One person starts a ceremony. Everyone else joins by scanning their screen. You need at least two devices.'));
  screen({
    title: 'Cahoots',
    menu: aboutSheet,
    body: [rows.length ? h('div', {}, h('p', { class: 'eyebrow' }, 'Your cahoots'), h('div', { class: 'rows' }, ...rows.map(row))) : empty],
    actions: [btn('Start a ceremony', () => go('#/new'), 'primary'), btn('Join one', () => go('#/join')), rows.length ? null : link('What is this?', aboutSheet)],
  });
}

function screenNew() {
  const params = { n: 2, pad_len: 65536, name: '', agent_seat: false };
  const name = input({ type: 'text', placeholder: 'e.g. sf-meetup', maxlength: '64', autocomplete: 'off', autocapitalize: 'off', oninput: (e) => { params.name = e.target.value; } });
  const sizeHint = h('span', { class: 'hint' }, SIZES[1][2]);
  const err = status();
  screen({
    title: 'New ceremony', back: '#/',
    body: [
      h('div', {},
        h('h1', {}, 'Set up the cahoot.'),
        h('p', { class: 'lead' }, 'Everyone will see these on their own screen.'),
        field('Name', name),
        field('People in the room', stepper(2, 8, params.n, (v) => { params.n = v; }), 'Count every device that will hold the secret, including yours.'),
        h('div', { class: 'field' }, h('label', {}, 'Secret size'), seg(SIZES.map(([v, l]) => [v, l]), params.pad_len, (v) => { params.pad_len = v; sizeHint.textContent = SIZES.find((s) => s[0] === v)[2]; }), sizeHint),
        h('details', { class: 'more' }, h('summary', {}, 'Advanced'),
          h('label', { class: 'toggle' }, h('span', {}, h('b', {}, 'Invite a hosted software member'), h('br'), h('span', { class: 'muted' }, 'A program in a secure enclave gets a seat and a copy of the secret. Leave off for a people-only cahoot.')), h('input', { type: 'checkbox', onchange: (e) => { params.agent_seat = e.target.checked; } }))),
        err),
    ],
    actions: [btn('Continue', () => {
      const n = params.name.trim();
      if (!n) { err.textContent = 'Give the cahoot a name.'; name.focus(); return; }
      go(`#/ceremony/${paramsHash({ ...params, name: n })}/pick`);
    }, 'primary')],
  });
  setTimeout(() => name.focus(), 50);
}

function screenJoin() {
  const err = status();
  const accept = (t) => {
    try {
      if (!t) throw 'That is not a Cahoots code.';
      if (t.kind === 'reveal') throw 'That is a step-2 animation. Join from someone still on step 1, or from the host.';
      if (t.kind === 'commit' && !t.params) throw 'That code has no setup in it. Scan the host instead.';
      const params = paramsFromBytes(t.kind === 'params' ? t.bytes : t.params);
      if (t.kind === 'commit') pendingCommits = [{ index: t.index, bytes: new Uint8Array(t.bytes) }];
      buzz();
      go(`#/ceremony/${paramsHash(params)}/pick`);
    } catch (e) { err.textContent = friendly(e); }
  };
  const take = (text) => accept(parseTagged(text));
  screen({
    title: 'Join a ceremony', back: '#/',
    menu: () => sheet(h('h2', {}, 'No camera?'), btn('Paste a code', () => pasteSheet('cahoots-commit:…', take)), fileBtn('Load a file', take)),
    body: [next('Point the camera at the screen of someone who has already joined. Hold still: their code is a short animation.'), fountainReceiver((bytes) => accept(parsePacked(bytes)), 'Fill the frame with their animation', take).el, err],
    actions: [link('No camera? Paste a code', () => pasteSheet('cahoots-commit:…', take))],
  });
}

/** Who you are, and where your randomness comes from. */
function screenPick(params) {
  const taken = new Set(pendingCommits.map((p) => p.index));
  let idx = [...Array(params.n).keys()].find((i) => !taken.has(i)) ?? 0;
  let src = 'csprng', extra = null;
  const err = status();
  const srcLine = h('span', { class: 'hint' }, 'Randomness: this device. The cahoot is labelled CSPRNG.');
  const seats = seg(Array.from({ length: params.n }, (_, i) => [i, i === 0 ? '1 · leader' : String(i + 1), taken.has(i)]), idx, (v) => { idx = Number(v); });
  const hardwareSheet = () => sheet((close) => {
    const file = h('input', { type: 'file', style: 'display:none', onchange: async (e) => {
      if (!e.target.files[0]) return;
      extra = new Uint8Array(await e.target.files[0].arrayBuffer());
      src = 'file'; srcLine.textContent = `Randomness: this device mixed with ${kib(extra.length)} from a file. Labelled TRNG.`; close();
    } });
    const r = fountainReceiver((bytes) => { extra = bytes; src = 'qr'; srcLine.textContent = `Randomness: this device mixed with ${kib(bytes.length)} by camera. Labelled TRNG.`; close(); }, 'Point at the laptop\'s animation');
    return h('div', { class: 'stack' },
      h('h2', {}, 'Add hardware randomness'),
      h('p', { class: 'muted' }, `Optional. A ${kib(params.pad_len)} file from cahoots-yk (a YubiKey) is mixed in with this device's own randomness, never used alone. It earns the cahoot the TRNG label.`),
      file, btn('Load a file', () => file.click()),
      h('p', { class: 'muted' }, 'Or scan the animation cahoots-yk shows on a laptop:'), r.el,
      extra ? btn('Remove hardware randomness', () => { extra = null; src = 'csprng'; srcLine.textContent = 'Randomness: this device. The cahoot is labelled CSPRNG.'; close(); }, 'danger') : null);
  });
  screen({
    title: params.name, sub: `${params.n} people · ${kib(params.pad_len)}`, back: '#/',
    menu: hardwareSheet,
    body: [
      h('div', {},
        h('h1', {}, 'Which seat are you?'),
        h('p', { class: 'lead' }, taken.size ? `Seat ${[...taken].map((i) => i + 1).join(', ')} is taken by the person you scanned. Pick a free one, and make sure nobody else picks the same.` : 'Each person takes a different seat. The person who set this up is seat 1.'),
        field('Your seat', seats),
        params.agent_seat ? notice('This cahoot invites a hosted software member. It will hold a copy of the secret in a secure enclave.') : null,
        srcLine, err),
    ],
    actions: [btn('Make my contribution', async () => {
      try {
        if (extra && extra.length !== params.pad_len) throw `The hardware file must be exactly ${params.pad_len} bytes, got ${extra.length}.`;
        const contribution = new Uint8Array(params.pad_len);
        for (let o = 0; o < contribution.length; o += 65536) crypto.getRandomValues(contribution.subarray(o, Math.min(o + 65536, contribution.length)));
        const c = new Ceremony(params.n, params.pad_len, params.name, params.agent_seat, idx, contribution, extra ?? undefined, crypto.getRandomValues(new Uint8Array(32)));
        ceremony = { c, params, label: extra ? 'TRNG' : 'CSPRNG', mode: 'show' };
        go(`#/ceremony/${paramsHash(params)}/run`);
      } catch (e) { err.textContent = friendly(e); }
    }, 'primary'), link('Add hardware randomness', hardwareSheet)],
  });
}

/** Commit and reveal: one stage with Show and Scan modes, member chips that update in place. */
function screenRun(params) {
  if (!ceremony) {
    screen({ title: 'Ceremony lost', body: [h('div', {}, h('h1', {}, 'This page was reloaded.'), h('p', { class: 'lead' }, 'A ceremony in progress lives only in memory, so it is gone. Nothing was kept. Join again from the start.'))], actions: [btn('Back to start', () => go('#/'), 'primary')] });
    return;
  }
  const { c } = ceremony;
  const me = c.index();
  for (const p of pendingCommits) { if (p.index !== me) { try { c.add_commitment(p.index, p.bytes); } catch (e) { /* shown as waiting */ } } }
  pendingCommits = [];
  const phase = c.phase();
  if (phase === 'aborted') {
    ceremony = null;
    screen({ title: 'Ceremony stopped', body: [h('div', {}, h('h1', {}, 'A reveal did not match its commitment.'), h('p', { class: 'lead' }, 'Nothing was kept. Either someone\'s contribution changed after they committed, or a code was corrupted. Start again from the beginning.'))], actions: [btn('Back to start', () => go('#/'), 'primary')] });
    return;
  }
  if (phase === 'complete') { screenRevealDone(params); return; }
  const rerender = () => screenRun(params);
  const st = status();
  const seatName = (i) => (i === me ? 'you' : `seat ${i + 1}`);

  // Member chips and the "waiting for" line update in place, so the camera keeps running between scans.
  const chips = h('div', { class: 'members' });
  const waiting = next('');
  const refresh = () => {
    if (c.phase() !== phase) { rerender(); return; }
    const missing = new Set(phase === 'commit' ? c.missing_commitments() : c.missing_reveals());
    chips.replaceChildren(...Array.from({ length: params.n }, (_, i) =>
      h('span', { class: `chip${i === me ? ' me' : missing.has(i) ? '' : ' on'}` }, i === me ? 'you' : `${i + 1}${missing.has(i) ? '' : ' ✓'}`)));
    const left = [...missing].filter((i) => i !== me);
    if (!left.length) { waiting.textContent = 'All in. Moving on.'; return; }
    waiting.textContent = phase === 'commit'
      ? `Show this to ${left.map(seatName).join(' and ')}, and scan theirs. Any order. Hold still: each one takes a moment.`
      : `Let ${left.map(seatName).join(' and ')} scan this animation. When they show you theirs, scan it.`;
  };
  const got = (i, what) => { buzz(); toast(`Got seat ${i + 1}'s ${what}`, 'ok'); refresh(); };
  // A sealed reveal that arrives before its commitment stands in for it; the core handles that.
  const accept = (t) => {
    try {
      if (!t || t.kind === 'params') return;
      if (t.index === me) { st.textContent = `That is seat ${me + 1}'s, which is yours. If someone else chose seat ${me + 1} too, one of you must leave and rejoin with another seat.`; return; }
      if (t.kind === 'commit') { c.add_commitment(t.index, t.bytes); got(t.index, 'commitment'); }
      else if (t.kind === 'reveal') { c.add_reveal(t.index, t.bytes); got(t.index, 'reveal'); }
    } catch (e) { st.textContent = friendly(e); if (c.phase() === 'aborted') rerender(); }
  };
  const feedText = (text) => accept(parseTagged(text));
  const feedBytes = (bytes) => accept(parsePacked(bytes));

  let show, text, fileName;
  // One camera for both steps: commitments and reveals are both animations now, and a member
  // who is ahead shows a reveal while this one is still collecting commitments.
  const scanStage = () => fountainReceiver(feedBytes, 'Fill the frame with their animation', feedText).el;
  if (phase === 'commit') {
    text = enc.commit(me, c.commitment(), c.params_bytes());
    fileName = `commit-${me + 1}.txt`;
    show = animatedQr(packCommit(me, c.params_bytes(), c.commitment()), `seat ${me + 1} · step 1`);
  } else {
    const reveal = revealOf(c);
    text = enc.reveal(me, reveal);
    fileName = `reveal-${me + 1}.txt`;
    show = animatedQr(packReveal(reveal), `seat ${me + 1} · step 2`);
  }
  const desktop = matchMedia('(min-width: 960px)').matches;
  const showWrap = h('div', { class: 'half', style: desktop || ceremony.mode === 'show' ? '' : 'display:none' }, h('p', { class: 'eyebrow' }, 'Show'), show);
  const scanWrap = h('div', { class: 'half', style: desktop || ceremony.mode === 'scan' ? '' : 'display:none' }, h('p', { class: 'eyebrow' }, 'Scan'), desktop || ceremony.mode === 'scan' ? scanStage() : null);
  const modeSeg = seg([['show', 'Show mine'], ['scan', 'Scan theirs']], ceremony.mode, (v) => {
    ceremony.mode = v;
    showWrap.style.display = v === 'show' ? '' : 'none';
    scanWrap.style.display = v === 'scan' ? '' : 'none';
    if (v !== 'scan') stopScanner(); else scanWrap.replaceChildren(h('p', { class: 'eyebrow' }, 'Scan'), scanStage());
  });
  if (desktop) modeSeg.classList.add('desktop-hide');
  screen({
    title: phase === 'commit' ? 'Step 1 of 3 · Commit' : 'Step 2 of 3 · Reveal', sub: params.name,
    back: () => { if (confirm('Leave the ceremony? Nothing is kept.')) { ceremony = null; go('#/'); } },
    menu: () => sheet(
      h('h2', {}, 'Share yours another way'),
      copyBtn(text), downloadBtn(fileName, text),
      h('h2', {}, 'Receive theirs another way'),
      btn('Paste a code', () => pasteSheet(phase === 'commit' ? 'cahoots-commit:…' : 'cahoots-reveal:…', feedText)),
      fileBtn('Load a file', feedText),
      btn('Leave the ceremony', () => { if (confirm('Leave the ceremony? Nothing is kept.')) { ceremony = null; go('#/'); } }, 'danger')),
    body: [chips, waiting, modeSeg, h('div', { class: 'pair' }, showWrap, scanWrap), st],
  });
  refresh();
}

/** Everything is in on this screen, but the others may still need this member's reveal. Show it until they say so. */
function screenRevealDone(params) {
  const { c } = ceremony;
  const me = c.index();
  const reveal = revealOf(c);
  const text = enc.reveal(me, reveal);
  const others = Array.from({ length: params.n }, (_, i) => i).filter((i) => i !== me).map((i) => `seat ${i + 1}`).join(' and ');
  screen({
    title: 'Step 2 of 3 · Reveal', sub: params.name,
    back: () => { if (confirm('Leave the ceremony? Nothing is kept.')) { ceremony = null; go('#/'); } },
    menu: () => sheet(h('h2', {}, 'Share your reveal another way'), copyBtn(text), downloadBtn(`reveal-${me + 1}.txt`, text)),
    body: [
      next(`Your screen has everything. ${others.charAt(0).toUpperCase() + others.slice(1)} still ${params.n > 2 ? 'need' : 'needs'} to scan this animation. Hold still until their bar fills.`),
      animatedQr(packReveal(reveal), `seat ${me + 1} · step 2`),
    ],
    actions: [btn(params.n > 2 ? 'Everyone has scanned it' : 'They have scanned it', () => screenWords(), 'primary')],
  });
}

function screenWords() {
  const { c, label, params } = ceremony;
  const me = c.index();
  const reveal = revealOf(c); // kept so members still collecting can scan it from here
  const out = c.finish(label);
  ceremony = null;
  const words = out.words().split(' ');
  const id = out.circle_id_hex();
  const revealText = enc.reveal(me, reveal);
  const showReveal = () => sheet(h('h2', {}, 'Someone still collecting? Show them this.'), animatedQr(packReveal(reveal), `seat ${me + 1} · step 2`), copyBtn(revealText), downloadBtn(`reveal-${me + 1}.txt`, revealText));
  screen({
    title: 'Step 3 of 3 · Check', sub: params.name,
    menu: showReveal,
    body: [
      next('One person reads these out loud. Everyone else checks their own screen shows the same six, in the same order.'),
      h('div', { class: 'panel' },
        h('div', { class: 'cap' }, h('span', {}, params.name), h('span', {}, `${label} · ${id.slice(0, 8)}`)),
        h('div', { class: 'words' }, ...words.map((w, i) => h('div', {}, h('small', {}, String(i + 1)), w)))),
    ],
    actions: [
      btn('They match', async () => { const circle = out.into_circle(); live.set(id, circle); await saveCircle(circle); toast('Saved. You are in cahoots.', 'ok'); go(`#/c/${id}`); }, 'primary'),
      btn('They differ · start over', () => { if (confirm('Throw this away and start again?')) go('#/'); }, 'danger'),
      link('Someone still needs my reveal', showReveal),
    ],
  });
}

// ---------------------------------------------------------------- a circle: hub, send, read, history, settings
const budget = (circle) => {
  const pct = Math.max(0, Math.min(100, Math.round((circle.remaining() / circle.slice_len()) * 100)));
  return h('div', { class: 'budget' }, h('div', { class: 'budget-bar' }, h('div', { style: `width:${pct}%` })), h('span', {}, `${kib(circle.remaining())} of your share left · ${pct}%`));
};
async function screenCircle(id) {
  const circle = await openCircle(id);
  if (!circle) { screen({ title: 'Cahoots', body: [h('div', { class: 'empty' }, h('h1', {}, 'This cahoot is not on this device.'))], actions: [btn('Home', () => go('#/'), 'primary')] }); return; }
  const label = circle.label();
  const low = circle.remaining() < circle.slice_len() / 10;
  screen({
    title: circle.name(), sub: `${label} · you are seat ${circle.my_slice() + 1} of ${circle.members()}`, back: '#/',
    menu: () => sheet(
      h('h2', {}, circle.name()),
      h('p', { class: 'muted' }, CLAIM[label]),
      h('p', { class: 'muted' }, 'Inside the cahoot, a message proves a member wrote it, not which member.'),
      h('p', { class: 'muted' }, 'History stays on this device. Secret bytes are erased as they are used, but erasure on phone storage is best effort.'),
      btn('Delete this cahoot from this device', async () => { if (confirm('Delete? The secret and the history on this device are removed. Other members keep theirs.')) { live.delete(id); await deleteCircle(id); go('#/'); } }, 'danger')),
    body: [
      h('div', {},
        budget(circle),
        low ? notice('Less than 10% of your share is left. Plan a new ceremony.') : null,
        h('div', { class: 'rows' },
          h('a', { class: 'row-item', href: `#/c/${id}/send` }, h('div', {}, h('b', {}, 'Send a message'), h('span', {}, 'short · unbreakable · shown as a code to scan or paste')), h('span', { class: 'arrow' }, '›')),
          h('a', { class: 'row-item', href: `#/c/${id}/read` }, h('div', {}, h('b', {}, 'Read a message'), h('span', {}, 'scan a code or paste one')), h('span', { class: 'arrow' }, '›')),
          h('a', { class: 'row-item', href: `#/chat/${id}` }, h('div', {}, h('b', {}, 'Chat'), h('span', {}, 'over a relay you run · end-to-end encrypted')), h('span', { class: 'arrow' }, '›')),
          h('a', { class: 'row-item', href: `#/c/${id}/hello` }, h('div', {}, h('b', {}, 'Hello words'), h('span', {}, 'on a call, know it is really them · two words each')), h('span', { class: 'arrow' }, '›')),
          h('a', { class: 'row-item', href: `#/c/${id}/history` }, h('div', {}, h('b', {}, 'History'), h('span', {}, 'messages on this device')), h('span', { class: 'arrow' }, '›')))),
    ],
  });
}

async function screenSend(id) {
  const circle = await openCircle(id);
  if (!circle) return go('#/');
  let type = 0, padOn = true;
  const text = h('textarea', { class: 'input text', placeholder: 'Up to 255 characters', maxlength: '255', rows: 4 });
  const count = h('span', { class: 'hint' }, '0 / 255');
  text.oninput = () => { count.textContent = `${utf8.encode(text.value).length} / 255`; };
  const err = status();
  screen({
    title: 'Send a message', sub: `${kib(circle.remaining())} of your share left`, back: `#/c/${id}`,
    menu: () => sheet(
      h('h2', {}, 'Message options'),
      field('Kind', seg(TYPES.map((t, i) => [i, t]), type, (v) => { type = Number(v); }), 'Text is the normal kind. The others are for programs that read the feed.'),
      h('label', { class: 'toggle' }, h('span', {}, h('b', {}, 'Hide the length'), h('br'), h('span', { class: 'muted' }, 'Pads to 32, 64, 128, or 256 bytes so the size gives nothing away.')), h('input', { type: 'checkbox', checked: padOn, onchange: (e) => { padOn = e.target.checked; } }))),
    body: [
      h('div', {},
        h('h1', {}, 'Write it.'),
        h('p', { class: 'lead' }, 'It is encrypted with secret bytes that are then erased. Nothing can read it except your cahoot.'),
        field('Message', text, count),
        err),
    ],
    actions: [btn('Encrypt', async () => {
      try {
        if (!text.value.trim()) { err.textContent = 'Write something first.'; text.focus(); return; }
        let pt = utf8.encode(text.value);
        if (padOn) { const p = bucket_pad(pt); if (!p) throw 'Too long for padding (255 bytes max). Shorten it, or turn padding off in the menu.'; pt = p; }
        const wire = circle.send(type, pt);
        await saveCircle(circle); // the range is spent in memory already; the wire leaves only after the ledger is saved
        await saveMessage({ circle: id, dir: 'out', type: TYPES[type], text: text.value, ts: Date.now() });
        screenShowWire(id, circle, wire, 32 + pt.length);
      } catch (e) { err.textContent = friendly(e); }
    }, 'primary')],
  });
  setTimeout(() => text.focus(), 50);
}
function screenShowWire(id, circle, wire, spent) {
  screen({
    title: 'Encrypted', sub: `${spent} bytes of the secret used · ${kib(circle.remaining())} left`, back: `#/c/${id}`,
    menu: () => sheet(h('h2', {}, 'Share another way'), copyBtn(wire), downloadBtn('message.txt', wire)),
    body: [next('Let them scan this, or send it as text. Post it anywhere: only your cahoot can read it.'), panelQr(wire, circle.name(), 'scan with Read a message')],
    actions: [btn('Done', () => go(`#/c/${id}`), 'primary'), link('Copy as text', async () => { await navigator.clipboard.writeText(wire); toast('Copied'); })],
  });
}
async function screenRead(id) {
  const circle = await openCircle(id);
  if (!circle) return go('#/');
  const st = status();
  const take = async (wire) => {
    if (wire.startsWith('cahoots-')) { st.textContent = 'That is a ceremony code, not a message.'; return; }
    try {
      const r = circle.receive(wire.trim());
      await saveCircle(circle);
      const pt = r.plaintext();
      const un = bucket_unpad(pt);
      const shown = utf8d.decode(un ?? pt);
      await saveMessage({ circle: id, dir: 'in', type: TYPES[r.msg_type()], text: shown, slice: r.slice(), ts: Date.now() });
      buzz();
      screen({
        title: 'Decrypted', sub: `from seat ${r.slice() + 1}${r.msg_type() ? ` · ${TYPES[r.msg_type()]}` : ''}`, back: `#/c/${id}`,
        body: [h('div', { class: 'panel' }, h('div', { class: 'cap' }, h('span', {}, circle.name()), h('span', {}, 'one-time pad')), h('p', { class: 'plain' }, shown))],
        actions: [btn('Read another', () => screenRead(id), 'primary'), btn('Done', () => go(`#/c/${id}`))],
      });
    } catch (e) { st.textContent = friendly(e); }
  };
  screen({
    title: 'Read a message', sub: circle.name(), back: `#/c/${id}`,
    menu: () => sheet(h('h2', {}, 'No camera?'), btn('Paste a message', () => pasteSheet('Paste the message text', take, 'Paste a message')), fileBtn('Load a file', take)),
    body: [next('Point the camera at the message code.'), camera(take, 'Fill the frame with the code'), st],
    actions: [link('No camera? Paste the text', () => pasteSheet('Paste the message text', take, 'Paste a message'))],
  });
}
async function screenHistory(id) {
  const circle = await openCircle(id);
  if (!circle) return go('#/');
  const msgs = (await loadMessages(id)).reverse();
  screen({
    title: 'History', sub: circle.name(), back: `#/c/${id}`,
    body: [msgs.length
      ? h('div', { class: 'top' }, ...msgs.map((m) => h('div', { class: 'msg' }, h('div', { class: 'meta' }, `${m.dir === 'out' ? 'you sent' : `from seat ${m.slice + 1}`}${m.type !== 'text' ? ` · ${m.type}` : ''} · ${new Date(m.ts).toLocaleString()}`), h('div', { class: 'plain' }, m.text))))
      : h('div', { class: 'empty' }, h('h1', {}, 'Nothing yet.'), h('p', { class: 'lead' }, 'Messages you send or read show up here.'))],
  });
}

// ---------------------------------------------------------------- hello words: pairwise, spoken, moved on demand
async function screenHello(id, peerStr) {
  const circle = await openCircle(id);
  if (!circle) return go('#/');
  const me = circle.my_slice();
  const settings = await loadSettings(id);
  const others = [...Array(circle.members()).keys()].filter((s) => s !== me);
  const tooOld = () => screen({
    title: 'Hello words', sub: circle.name(), back: `#/c/${id}`,
    body: [h('div', { class: 'empty' }, h('h1', {}, 'This cahoot is from before hello words.'), h('p', { class: 'lead' }, 'The words come from keys made during the ceremony. Run a new ceremony and they will be there.'))],
    actions: [btn('Back', () => go(`#/c/${id}`), 'primary')],
  });
  if (!others.some((s) => circle.has_pair_key(s))) return tooOld();
  if (peerStr === undefined && others.length === 1) peerStr = String(others[0]);
  if (peerStr === undefined) {
    screen({
      title: 'Hello words', sub: circle.name(), back: `#/c/${id}`,
      body: [
        next('Who are you talking to?'),
        h('div', { class: 'rows' }, ...others.map((s) => h('a', { class: 'row-item', href: `#/c/${id}/hello/${s}` },
          h('div', {}, h('b', {}, nameOf(settings, s)), h('span', {}, circle.has_pair_key(s) ? `hello #${circle.pair_counter(s) + 1}` : 'no key with this seat')),
          h('span', { class: 'arrow' }, '›')))),
      ],
    });
    return;
  }
  const peer = Number(peerStr);
  if (!circle.has_pair_key(peer)) return tooOld();
  const who = () => nameOf(settings, peer);
  const render = () => {
    const w = circle.hello_words(peer);
    const n = w.counter() + 1;
    screen({
      title: 'Hello words', sub: `with ${who()} · #${n}`, back: others.length === 1 ? `#/c/${id}` : `#/c/${id}/hello`,
      menu: () => sheet((close) => {
        const nameIn = input({ type: 'text', value: settings.names?.[peer] || '', placeholder: `Seat ${peer + 1}`, maxlength: '24', autocomplete: 'off' });
        const numIn = input({ type: 'number', value: String(n), min: '1', max: '4294967295', inputmode: 'numeric' });
        return h('div', { class: 'stack' },
          h('h2', {}, 'How hello words work'),
          h('p', { class: 'muted' }, `Only your two devices can make these words: they come from a key that you and ${who()} made together during the ceremony. A voice clone cannot say them. Each hello number has its own words, and every word is said once.`),
          h('p', { class: 'muted' }, 'If you are on different numbers, set the same one here on both devices.'),
          field('Their name on this device', nameIn),
          field('Hello number', numIn),
          btn('Save', async () => {
            const names = { ...(settings.names || {}) };
            if (nameIn.value.trim()) names[peer] = nameIn.value.trim(); else delete names[peer];
            settings.names = names;
            await saveSettings({ ...settings, id });
            const num = Math.max(1, Math.floor(Number(numIn.value) || 1));
            if (num !== n) { circle.set_pair_counter(peer, num - 1); await saveCircle(circle); }
            close(); render();
          }, 'primary'));
      }),
      body: [
        next(`Say yours. If ${who()} answers with theirs, it is really them.`),
        h('div', { class: 'panel' },
          h('div', { class: 'cap' }, h('span', {}, `${circle.name()} · hello #${n}`), h('span', {}, `seat ${me + 1} ↔ seat ${peer + 1}`)),
          h('div', { class: 'hello' },
            h('div', { class: 'who' }, 'You say'), h('div', { class: 'pair' }, w.mine()),
            h('div', { class: 'who' }, `${who()} says back`), h('div', { class: 'pair' }, w.theirs()))),
        h('p', { class: 'muted center' }, 'Something feel off? Both of you tap Next hello. If they cannot say the new words, that is not them.'),
      ],
      actions: [
        btn('Next hello', async () => { circle.next_hello(peer); await saveCircle(circle); buzz(); toast(`Now on hello #${circle.pair_counter(peer) + 1}. Make sure ${who()} moves too.`); render(); }, 'primary'),
      ],
    });
  };
  render();
}

// ---------------------------------------------------------------- chat (pad-seeded, over the Numbers Station)
let chatRun = 0; // bumps on every render; a stale poll loop sees a different value and stops
window.addEventListener('hashchange', () => { chatRun++; });
async function screenChat(id) {
  const circle = await openCircle(id);
  if (!circle) return go('#/');
  const me = circle.my_slice();
  const settings = await loadSettings(id);
  const run = ++chatRun;
  const relayBase = () => (settings.relay || '').trim().replace(/\/$/, '');
  const st = status();
  const list = h('div', { class: 'chatlist' });
  const inp = input({ type: 'text', placeholder: 'Message', maxlength: '4000', autocomplete: 'off', enterkeyhint: 'send' });
  const sendBtn = btn('Send', () => doSend(), 'primary');
  const epoch = () => { const e = circle.current_epoch(); return e ? `epoch ${e}` : 'no epoch yet'; };
  const bubble = (m) => m.dir === 'rejected'
    ? h('div', { class: 'bubble rejected' }, `rejected · ${friendly(m.reason)} · seq ${m.seq}`)
    : h('div', { class: `bubble ${m.dir}` }, h('div', { class: 'meta' }, `${m.dir === 'out' ? 'you' : nameOf(settings, m.sender)} · ${new Date(m.ts).toLocaleTimeString()}`), h('div', {}, m.text));
  const append = (m) => { list.append(bubble(m)); list.scrollTop = list.scrollHeight; };
  const history = await loadChat(id);
  for (const m of history) append(m);
  const titleSub = h('small', {}, `${CHAT_LABEL} · ${epoch()}`);

  const relaySheet = () => sheet((close) => {
    const relayIn = input({ type: 'url', value: settings.relay, placeholder: 'https://relay.example.com', autocapitalize: 'off', autocomplete: 'off' });
    return h('div', { class: 'stack' },
      h('h2', {}, 'Relay'),
      h('p', { class: 'muted' }, 'Chat needs a Numbers Station relay that every member can reach. It only ever sees encrypted messages. Run one with cahoots-station, or use one a member runs.'),
      field('Relay address', relayIn),
      btn('Save and connect', async () => { settings.relay = relayIn.value.trim(); await saveSettings({ id, relay: settings.relay }); close(); chatRun++; screenChat(id); }, 'primary'));
  });

  const post = async (bytes) => {
    const res = await fetch(`${relayBase()}/v1/circles/${id}/messages`, { method: 'POST', headers: { 'Content-Type': 'application/octet-stream' }, body: bytes });
    if (!res.ok) throw `relay ${res.status}: ${await res.text()}`;
    return res.json();
  };
  const doSend = async () => {
    const text = inp.value;
    if (!text) return;
    if (!relayBase()) { relaySheet(); return; }
    sendBtn.disabled = true;
    try {
      const pt = chat_bucket_pad(utf8.encode(text));
      if (!pt) throw 'Message too long.';
      const wire = circle.chat_send(pt);
      await saveCircle(circle); // keys and any epoch spend are recorded before the ciphertext leaves
      await post(b64_decode(wire));
      const m = { circle: id, dir: 'out', sender: me, epoch: circle.current_epoch(), text, ts: Date.now() };
      await saveChat(m); append(m); titleSub.textContent = `${CHAT_LABEL} · ${epoch()}`;
      inp.value = '';
    } catch (e) { st.textContent = `Not sent: ${friendly(e)}`; }
    sendBtn.disabled = false;
    inp.focus();
  };
  inp.onkeydown = (e) => { if (e.key === 'Enter') doSend(); };

  const poll = async () => {
    while (run === chatRun) {
      const after = circle.relay_seq();
      const url = `${relayBase()}/v1/circles/${id}/messages?wait=${POLL_WAIT_MS}${after == null ? '' : `&after=${after}`}`;
      let data;
      try {
        const res = await fetch(url);
        if (!res.ok) throw `${res.status}`;
        data = await res.json();
        st.textContent = `connected · last seq ${circle.relay_seq() ?? 'none'}`;
      } catch (e) {
        st.textContent = 'Relay unreachable. Retrying.';
        await new Promise((r) => setTimeout(r, 3000));
        continue;
      }
      if (run !== chatRun) break;
      let changed = false;
      for (const entry of data.entries || []) {
        const bytes = b64_decode(entry.message);
        if (bytes[0] === 2 && bytes[22] !== me) {
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
        circle.set_relay_seq(BigInt(entry.seq));
        changed = true;
      }
      if (changed) { await saveCircle(circle); titleSub.textContent = `${CHAT_LABEL} · ${epoch()}`; }
    }
  };

  const noRelay = h('div', { class: 'empty' },
    h('h1', {}, 'Chat needs a relay.'),
    h('p', { class: 'lead' }, 'A relay is a small server any member can run. It carries encrypted messages between you and cannot read them. Set its address to start.'),
    btn('Set the relay address', relaySheet, 'primary'));
  const el = screen({
    title: circle.name(), back: `#/c/${id}`,
    menu: () => sheet((close) =>
      h('div', { class: 'stack' },
        h('h2', {}, 'Chat'),
        h('p', { class: 'muted' }, CHAT_CLAIM),
        h('p', { class: 'muted' }, `Starting a new epoch rotates everyone's keys and costs 32 secret bytes. ${kib(circle.remaining())} of your share left. The seat shown on a message is a chain label, not a proof of who typed.`),
        btn('Start a new epoch', async () => { try { circle.rotate_epoch(); await saveCircle(circle); titleSub.textContent = `${CHAT_LABEL} · ${epoch()}`; toast('New epoch started', 'ok'); close(); } catch (e) { st.textContent = friendly(e); } }),
        btn('Relay address', () => { close(); relaySheet(); }))),
    body: relayBase() ? [history.length ? null : h('p', { class: 'muted center' }, 'No messages yet.'), list, st] : [noRelay],
    actions: relayBase() ? [h('div', { class: 'composer' }, inp, sendBtn)] : [],
  });
  el.querySelector('.bar .title').append(titleSub);
  if (relayBase()) poll();
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
    } else if (parts[0] === 'c') {
      if (parts[2] === 'send') await screenSend(parts[1]);
      else if (parts[2] === 'read') await screenRead(parts[1]);
      else if (parts[2] === 'history') await screenHistory(parts[1]);
      else if (parts[2] === 'hello') await screenHello(parts[1], parts[3]);
      else await screenCircle(parts[1]);
    } else if (parts[0] === 'chat') await screenChat(parts[1]);
    else await screenHome();
  } catch (e) { screen({ title: 'Cahoots', body: [h('div', { class: 'empty' }, h('h1', {}, 'Something went wrong.'), h('p', { class: 'lead' }, friendly(e)))], actions: [btn('Home', () => go('#/'), 'primary')] }); }
}

await init();
window.addEventListener('hashchange', route);
if ('serviceWorker' in navigator && location.protocol !== 'file:') navigator.serviceWorker.register('./sw.js').catch(() => {});
route();
