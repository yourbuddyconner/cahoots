// Cahoots phone/laptop client. A ceremony with codes only and pure-pad messaging make no network calls.
// A ceremony through the relay, and the chat screen, talk to a Numbers Station relay over HTTP; they
// send and receive ciphertext only.
// All crypto is in cahoots-core and cahoots-chat (WASM).
//
// UI rules: one job per screen. A top bar, one stage element, one primary action. Every screen says
// what to do next in one line. Nothing the person needs is behind a scroll.
import init, {
  Ceremony, Circle, FountainEncoder, FountainDecoder, offer_params,
  b64_encode, b64_decode, bucket_pad, bucket_unpad, chat_bucket_pad, chat_bucket_unpad, qr_svg, qr_decode_rgba,
} from './pkg/cahoots_wasm.js';

const CLAIM = {
  TRNG: 'Secure against unlimited computing power, as long as no pad byte is reused.',
  CSPRNG: 'Computationally secure. No hardware randomness went into this pad.',
};
const ERRORS = {
  reuse: 'Possible pad reuse: this range was already spent. Refused, nothing changed.',
  tag: 'Integrity check failed. Refused, nothing spent.',
  'wrong-circle': 'This message belongs to a different circle.',
  'out-of-range': 'The message points outside the pad.',
  exhausted: 'Your share of the pad is used up. Hold a new ceremony.',
  base64url: 'That is not a Cahoots message.',
  replay: 'Already received.',
  'too-far-ahead': 'Too many messages were skipped before this one.',
  'not-synced': 'Still reading the relay. Try again in a moment.',
  'unknown-epoch': 'This message is from an epoch whose keys this device no longer keeps.',
  'pad-epoch': 'This message uses the old pad-keyed chat, which this app no longer reads.',
  'no-root': 'This circle was made before root epochs. Run a new ceremony to chat.',
  version: 'Not a chat message.',
  length: 'Malformed message.',
  'wrong-phase': 'Not yet. Every commitment has to be in first.',
  'bad-tier': 'A member declared a device tier this app does not know. Ask them to update.',
  'bad-seed': 'This phone drew an unusable signing key. Try again.',
  aborted: 'This ceremony was stopped.',
  'not-an-offer': 'That is not a host\'s code.',
  offer: 'That host\'s code does not read. Scan it again.',
  'host-seat': 'Seat 1 is the host\'s. Pick another seat.',
  'not-a-join-code': 'That is not a joiner\'s code.',
  'other-ceremony': 'That code is for another ceremony.',
  'not-host': 'Only the host scans a joiner\'s code.',
};
const friendly = (e) => {
  const s = String(e);
  if (ERRORS[s]) return ERRORS[s];
  const m = s.match(/^(conflicting-commit|commit-mismatch|bad-index):(\d+)$/);
  if (m && m[1] === 'conflicting-commit') return `Two different people are using seat ${Number(m[2]) + 1}. One of them needs to leave and rejoin with another seat.`;
  if (m && m[1] === 'commit-mismatch') return `Seat ${Number(m[2]) + 1}'s reveal does not match their commitment.`;
  if (m && m[1] === 'bad-index') return 'That code is for a different group size.';
  const m2 = s.match(/^(corrupted|bad-key|bad-attestation):(\d+)$/);
  if (m2 && m2[1] === 'corrupted') return 'That scan came through garbled. Scan it again.';
  if (m2 && m2[1] === 'bad-key') return `Seat ${Number(m2[2]) + 1}'s code is not valid. Ask them to show it again.`;
  if (m2 && m2[1] === 'bad-attestation') return `Seat ${Number(m2[2]) + 1}'s signature does not check out. Ask them to show it again.`;
  return s;
};
const CHAT_LABEL = 'End-to-end encrypted';
const CHAT_CLAIM = 'The circle root from your ceremony seeds the keys and ChaCha20-Poly1305 encrypts. The relay sees ciphertext and traffic shape only.';
// The relay the app ships with. A circle can point at another one from the chat menu.
const DEFAULT_RELAY = 'https://relay.cahoots.network';
const POLL_WAIT_MS = 25000;
const CEREMONY_POLL_MS = 2000; // a ceremony through the relay reads the relay every 2 seconds
const LOST = 'This ceremony was lost when the app closed. Everyone starts over with a new code.';
/** A ceremony in memory is gone after a reload or a close. Joining it again cannot work. */
const ceremonyLost = () => screen({ title: 'Ceremony lost', body: [h('div', {}, h('h1', {}, 'This ceremony is gone.'), h('p', { class: 'lead' }, LOST), h('p', { class: 'muted' }, 'Nothing was kept.'))], actions: [btn('Back to start', () => go('#/'), 'primary')] });
const OFFER_PREFIX = 'cahoots:ceremony:v1:';
const JOINED_PREFIX = 'cahoots:joined:v1:';
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
// A circle saved before the app shipped a default relay has an empty address; it gets the default.
const loadSettings = async (id) => { const s = (await tx('settings', 'readonly', (s) => s.get(id))) || { id }; if (!s.relay) s.relay = DEFAULT_RELAY; return s; };
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
let routeGeneration = 0;
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
  const bar = h('header', { class: 'bar' },
    back ? h('button', { 'aria-label': 'Back', onclick: () => (typeof back === 'function' ? back() : go(back)) }, '‹') : h('span'),
    h('div', { class: 'title' }, title, sub ? h('small', {}, sub) : null),
    menu ? h('button', { 'aria-label': 'More options', onclick: menu }, '⋯') : h('span'));
  const stage = h('div', { class: 'stage' }, ...body);
  const el = h('div', { class: 'screen' }, bar, h('main', { class: 'body' }, stage), actions.length ? h('div', { class: 'actions' }, ...actions) : null);
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
let fieldNumber = 0;
function field(label, control, hint) {
  const id = control.id || `field-${++fieldNumber}`;
  control.id = id;
  const labelId = `${id}-label`;
  const isInput = control.matches('input, textarea, select');
  if (!isInput) control.setAttribute('aria-labelledby', labelId);
  if (hint) control.setAttribute('aria-describedby', `${id}-hint`);
  return h('div', { class: 'field' }, h(isInput ? 'label' : 'span', { id: labelId, for: isInput ? id : null, class: isInput ? null : 'field-label' }, label), control, hint ? h('span', { class: 'hint', id: `${id}-hint` }, hint) : null);
}
const input = (attrs) => h('input', { class: 'input', ...attrs });
function stepper(min, max, value, onChange) {
  const out = h('output', {}, String(value));
  const set = (v) => { value = Math.min(max, Math.max(min, v)); out.textContent = String(value); onChange(value); };
  return h('div', { class: 'stepper' }, h('button', { type: 'button', onclick: () => set(value - 1), 'aria-label': 'Fewer' }, '−'), out, h('button', { type: 'button', onclick: () => set(value + 1), 'aria-label': 'More' }, '+'));
}
/** A bottom sheet for the secondary options a screen needs but shouldn't show by default. */
function sheet(...kids) {
  const bg = h('div', { class: 'sheet-bg', onclick: (e) => { if (e.target === bg) close(); } });
  const close = () => { document.removeEventListener('keydown', onKey); bg.classList.remove('on'); setTimeout(() => bg.remove(), 180); };
  const onKey = (e) => { if (e.key === 'Escape') { close(); document.removeEventListener('keydown', onKey); } };
  document.addEventListener('keydown', onKey);
  bg.addEventListener('click', e => { if (e.target.closest('a[href^="#"]')) close(); });
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
  h('h2', {}, 'A home for your circles'),
  h('p', { class: 'muted' }, 'Meet in person to create a circle, then come back here to chat, check someone on a call, or exchange a private note. No account is needed.'),
  h('p', { class: 'muted' }, 'Your circles, keys, and message history are saved in this browser. A relay delivers encrypted chat without receiving the keys. Clearing browser data can remove your circles.'),
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
    if (onTagged && (text.startsWith('cahoots-') || text.startsWith(OFFER_PREFIX))) { onTagged(text); return; }
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
// Text forms, for paste and files. A commitment is 32 bytes of hash, the member's 1216-byte KEM
// public key, its 33-byte seat public key and its tier; it carries the circle parameters too, so
// scanning any member is enough to join. A reveal is a sealed blob, encrypted to every other member
// (crates/core/src/reveal.rs). An attestation is the member's signature over the manifest.
const enc = {
  params: (bytes) => `cahoots-params:${b64_encode(bytes)}`,
  commit: (i, bytes, params) => `cahoots-commit:${i}:${b64_encode(bytes)}${params ? ':' + b64_encode(params) : ''}`,
  reveal: (i, bytes) => `cahoots-reveal:${i}:${b64_encode(bytes)}`,
  attest: (i, bytes) => `cahoots-attest:${i}:${b64_encode(bytes)}`,
};
// Binary forms, for the animated code: a type byte, then the same content.
const PACK = { commit: 1, reveal: 2, attest: 3 };
function packCommit(i, params, bytes) {
  const out = new Uint8Array(3 + params.length + bytes.length);
  out[0] = PACK.commit; out[1] = i; out[2] = params.length;
  out.set(params, 3); out.set(bytes, 3 + params.length);
  return out;
}
function packReveal(bytes) { const out = new Uint8Array(bytes.length + 1); out[0] = PACK.reveal; out.set(bytes, 1); return out; }
function packAttest(i, bytes) { const out = new Uint8Array(bytes.length + 2); out[0] = PACK.attest; out[1] = i; out.set(bytes, 2); return out; }
/** Decode an animated transfer into the same shape parseTagged gives. */
function parsePacked(bytes) {
  if (bytes[0] === PACK.commit) { const plen = bytes[2]; return { kind: 'commit', index: bytes[1], params: bytes.subarray(3, 3 + plen), bytes: bytes.subarray(3 + plen) }; }
  if (bytes[0] === PACK.reveal) { return { kind: 'reveal', index: bytes[2], params: null, bytes: bytes.subarray(1) }; }
  if (bytes[0] === PACK.attest) { if (bytes.length < 3) return null; return { kind: 'attest', index: bytes[1], params: null, bytes: bytes.subarray(2) }; }
  return null;
}
/** Our sealed reveal. Built once by the core; the randomness is only drawn the first time. */
const revealOf = (c) => c.reveal(crypto.getRandomValues(new Uint8Array(c.reveal_randomness_len())));
/** A phone declares tier 0: its keys live in ordinary storage. */
const PHONE_TIER = 0;
function parseTagged(text) {
  const m = text.trim().match(/^cahoots-(params|commit|reveal|attest):(?:(\d+):)?([A-Za-z0-9_-]+)(?::([A-Za-z0-9_-]+))?$/);
  if (!m) return null;
  if ((m[1] === 'attest' || m[1] === 'reveal') && m[2] == null) return null; // these always name their seat
  return { kind: m[1], index: m[2] == null ? null : Number(m[2]), bytes: b64_decode(m[3]), params: m[4] ? b64_decode(m[4]) : null };
}
function paramsFromBytes(b) {
  if (b[0] !== 1) throw 'unknown version';
  return { n: b[1], pad_len: new DataView(b.buffer, b.byteOffset).getUint32(2, true), agent_seat: b[6] === 1, name: utf8d.decode(b.subarray(8, 8 + b[7])) };
}
const paramsHash = (params) => b64_encode(utf8.encode(JSON.stringify(params)));
let pendingCommits = [];   // commitments scanned before this device's ceremony object exists
let pendingOffer = null;   // the host's code, read before this device picks a seat
let ceremony = null;       // in-memory only: { c, params, label, mode, attests }

// ================================================================ screens
// Public circle IDs provide a decorative mark, never a personal identity or credential.
function circleMark(id, name, className = '') {
  const turn = parseInt(id.slice(0, 2), 16) || 0;
  return h('span', { class: `circle-mark ${className}`, 'aria-hidden': 'true', style: `--turn:${turn % 90}deg` },
    h('span', { class: 'mark-ring' }), h('span', { class: 'mark-letter' }, (name || 'C').slice(0, 1).toUpperCase()));
}
function walletScreen({ className = '', ...options }) {
  const el = screen(options);
  el.classList.add('content-screen');
  if (className) el.classList.add(className);
  return el;
}
const toolLink = (href, symbol, title, detail) => h('a', { class: 'tool-card', href, 'aria-label': title },
  h('span', { class: 'tool-symbol', 'aria-hidden': 'true' }, symbol),
  h('div', {}, h('b', {}, title), h('span', {}, detail)), h('span', { class: 'tool-arrow', 'aria-hidden': 'true' }, '↗'));

async function screenHome() {
  const generation = routeGeneration;
  const circles = (await loadCircles()).sort((a, b) => (b.updated || 0) - (a.updated || 0));
  if (generation !== routeGeneration) return;
  walletScreen({ title: 'Cahoots', className: 'entry-screen', menu: aboutSheet,
    body: [h('header', { class: 'entry-heading' }, h('img', { src: 'icon.svg', alt: '', width: 38, height: 38 }),
      h('h1', {}, circles.length ? 'Your circles' : 'Meet in person. Stay in cahoots.'),
      h('p', { class: 'lead' }, circles.length ? 'Choose a circle to pick up the conversation.' : 'Create a private circle with the people beside you. Then keep talking, wherever you go.')),
      circles.length ? h('div', { class: 'entry-list' }, ...circles.map(c => h('a', { class: 'row-item', href: `#/c/${c.id}`, 'aria-label': c.name }, circleMark(c.id, c.name), h('div', {}, h('b', {}, c.name), h('span', {}, `${c.members || '—'} people · on this device`)), h('span', { 'aria-hidden': 'true' }, '›')))) : null,
      h('div', { class: 'entry-actions' }, btn('Create a circle', () => go('#/new'), 'primary'), btn('Join a circle', () => go('#/join'))),
      h('p', { class: 'device-note' }, 'Circles and history are saved in this browser. No account needed.')],
  });
}

function circleTools(circle) {
  const id = circle.circle_id_hex();
  return [
    toolLink(`#/c/${id}/hello`, '◌', 'Hello words', 'Check someone on a call.'),
    toolLink(`#/c/${id}/send`, '↗', 'Send a message', 'Encrypt a short note to share by code or text.'),
    toolLink(`#/c/${id}/read`, '↙', 'Read a message', 'Open a note from someone in this circle.'),
    toolLink(`#/c/${id}/history`, '≡', 'History', 'Notes sent and read on this device.'),
  ];
}

async function workspaceScreen(circle, settings, { view = 'conversation', title, content, aside = null, menu, isCurrent = () => true }) {
  const requestedHash = location.hash;
  const id = circle.circle_id_hex();
  const circles = await loadCircles();
  if (requestedHash !== location.hash || !isCurrent()) return null;
  const select = h('select', { class: 'circle-switcher', 'aria-label': 'Switch circle', onchange: e => go(`#/c/${e.target.value}`) },
    ...circles.map(c => h('option', { value: c.id, selected: c.id === id }, c.name)));
  const pages = [ ['conversation', '', 'Conversation'], ['people', '/people', 'People'], ['network', '/network', 'Network'] ];
  const navigation = className => h('nav', { class: className, 'aria-label': 'Circle sections' }, ...pages.map(([key, path, name]) => h('a', { href: `#/c/${id}${path}`, 'aria-current': key === view ? 'page' : null }, name)));
  const details = () => sheet(close => h('div', { class: 'stack' }, h('h2', {}, 'Circle details'), h('p', { class: 'muted' }, `${circle.name()} · ${circle.members()} people · your seat: ${circle.my_slice() + 1}`), budget(circle), h('p', { class: 'muted' }, 'This browser holds your circle’s secret. Chat history stays here. Browser erasure is best effort.'), copyBtn(id, 'Copy circle ID'), btn('Remove circle from this browser', async () => {
    if (!confirm('Remove this circle from this browser? Other members keep theirs.')) return;
    live.delete(id); chatDrafts.delete(id); await deleteCircle(id); close(); go('#/');
  }, 'danger')));
  const tools = () => sheet(close => h('div', { class: 'stack' }, h('h2', {}, 'Circle tools'), ...circleTools(circle), btn('Storage & security', () => { close(); details(); }), h('a', { class: 'link', href: `${SITE}/docs/#/apps`, target: '_blank', rel: 'noopener' }, 'More about circle tools ↗')));
  const rail = h('aside', { class: 'dashboard-rail' },
    h('a', { class: 'rail-brand', href: '#/', 'aria-label': 'All circles' }, h('img', { src: 'icon.svg', alt: '', width: 25, height: 25 }), 'cahoots'),
    h('label', { class: 'rail-label' }, 'YOUR CIRCLE', select), navigation('dashboard-navigation'),
    h('p', { class: 'rail-label' }, 'CIRCLE TOOLS'), h('div', { class: 'rail-tools' }, ...circleTools(circle)),
    h('div', { class: 'rail-footer' }, btn('Storage & security', details), h('a', { href: '#/' }, 'All circles')));
  const el = screen({ title: circle.name(), sub: `${circle.members()} people · on this device`, back: view === 'conversation' ? '#/' : `#/c/${id}`, menu: menu || details,
    body: [navigation('dashboard-mobile-tabs'), h('div', { class: `dashboard-grid${aside ? ' has-aside' : ''}` }, rail,
      h('section', { class: `workspace-pane ${view === 'conversation' ? 'conversation-pane' : ''}`, 'aria-label': title || 'Conversation' }, ...content), aside)],
  });
  el.classList.add('circle-dashboard');
  el.dataset.view = view;
  return { el, tools };
}

async function screenNetwork(id) {
  const generation = routeGeneration;
  const circle = await openCircle(id);
  if (!circle) return go('#/');
  const settings = await loadSettings(id);
  await workspaceScreen(circle, settings, { view: 'network', title: 'Network', isCurrent: () => generation === routeGeneration, content: [
    h('header', { class: 'workspace-heading' }, h('div', {}, h('h1', {}, 'Network'), h('p', {}, 'Private connections between machines in this circle.')),
      h('a', { class: 'btn primary compact-button', href: `${SITE}/docs/#/adapters`, target: '_blank', rel: 'noopener' }, 'Set up WireGuard ↗')),
    h('div', { class: 'network-notice' }, h('strong', {}, 'Desktop integration'), h('span', {}, 'The WireGuard CLI is available. This browser does not yet load its peer list or control tunnels.')),
    h('div', { class: 'network-table-wrap' }, h('table', { class: 'network-table' }, h('thead', {}, h('tr', {}, ...['Machine', 'Address', 'Connection', 'Managed by'].map(name => h('th', { scope: 'col' }, name)))),
      h('tbody', {}, h('tr', {}, h('td', { colspan: 4 }, h('div', { class: 'network-empty' }, h('span', { 'aria-hidden': 'true' }, '⌁'), h('h2', {}, 'No peer data in this browser'), h('p', {}, 'Use the desktop tool to announce machines and exchange tunnel keys. No machines or connection states are inferred from circle membership.'))))))),
    h('section', { class: 'network-setup' }, h('h2', {}, 'Start with the desktop tool'), h('ol', {},
      h('li', {}, h('b', {}, 'Set up the circle provider.'), h('p', {}, 'The CLI needs its own provider configuration. A saved browser circle is not automatically connected to it.')),
      h('li', {}, h('b', {}, 'Announce machines and exchange keys.'), h('p', {}, 'Use cahoots-wg on each machine. The integration guide covers the available commands.')),
      h('li', {}, h('b', {}, 'Manage tunnels on those machines.'), h('p', {}, 'Browser-based device enrollment and live tunnel controls are still to be built.')))),
  ] });
}

function screenNew() {
  const params = { n: 2, pad_len: 65536, name: '', agent_seat: false, via: 'relay' };
  const VIA = { relay: 'Everyone scans one code from your screen. The rest goes through the relay.', codes: 'Everyone shows and scans codes. No network is needed.' };
  const viaHint = h('span', { class: 'hint' }, VIA.relay);
  const name = input({ type: 'text', placeholder: 'e.g. sf-meetup', maxlength: '64', autocomplete: 'off', autocapitalize: 'off', oninput: (e) => { params.name = e.target.value; } });
  const sizeHint = h('span', { class: 'hint' }, SIZES[1][2]);
  const err = status();
  screen({
    title: 'Create a circle', back: '#/',
    body: [
      h('div', {},
        h('h1', {}, 'Bring your people together.'),
        h('p', { class: 'lead' }, 'Give your circle a name, then exchange codes with the people beside you.'),
        field('Name', name),
        field('People in the room', stepper(2, 8, params.n, (v) => { params.n = v; }), 'Count every device that will hold the secret, including yours.'),
        h('div', { class: 'field' }, h('label', {}, 'How to hold the ceremony'), seg([['relay', 'Through the relay'], ['codes', 'Codes only']], params.via, (v) => { params.via = v; viaHint.textContent = VIA[v]; }), viaHint),
        h('div', { class: 'field' }, h('label', {}, 'Secret size'), seg(SIZES.map(([v, l]) => [v, l]), params.pad_len, (v) => { params.pad_len = v; sizeHint.textContent = SIZES.find((s) => s[0] === v)[2]; }), sizeHint),
        h('details', { class: 'more' }, h('summary', {}, 'Advanced'),
          h('label', { class: 'toggle' }, h('span', {}, h('b', {}, 'Invite a hosted software member'), h('br'), h('span', { class: 'muted' }, 'A program in a secure enclave gets a seat and a copy of the secret. Leave off for a people-only circle.')), h('input', { type: 'checkbox', onchange: (e) => { params.agent_seat = e.target.checked; } }))),
        err),
    ],
    actions: [btn('Continue', () => {
      const n = params.name.trim();
      if (!n) { err.textContent = 'Give the circle a name.'; name.focus(); return; }
      pendingOffer = null; pendingCommits = [];
      go(`#/ceremony/${paramsHash({ ...params, name: n })}/pick`);
    }, 'primary')],
  });
  setTimeout(() => name.focus(), 50);
}

function screenJoin() {
  const err = status();
  // The host's code for a ceremony through the relay: pick a seat, and the rest goes through the relay.
  const acceptOffer = (text) => {
    try {
      const params = { ...paramsFromBytes(offer_params(text.trim())), via: 'relay', join: true };
      pendingOffer = text.trim(); pendingCommits = [];
      buzz();
      go(`#/ceremony/${paramsHash(params)}/pick`);
    } catch (e) { err.textContent = friendly(e); }
  };
  const accept = (t) => {
    try {
      if (!t) throw 'That is not a Cahoots code.';
      if (t.kind === 'reveal' || t.kind === 'attest') throw `That is a step-${t.kind === 'reveal' ? 2 : 3} animation. Join from someone still on step 1, or from the host.`;
      if (t.kind === 'commit' && !t.params) throw 'That code has no setup in it. Scan the host instead.';
      const params = paramsFromBytes(t.kind === 'params' ? t.bytes : t.params);
      if (t.kind === 'commit') pendingCommits = [{ index: t.index, bytes: new Uint8Array(t.bytes) }];
      pendingOffer = null;
      buzz();
      go(`#/ceremony/${paramsHash(params)}/pick`);
    } catch (e) { err.textContent = friendly(e); }
  };
  const take = (text) => (text.trim().startsWith(OFFER_PREFIX) ? acceptOffer(text) : accept(parseTagged(text)));
  screen({
    title: 'Join a ceremony', back: '#/',
    menu: () => sheet(h('h2', {}, 'No camera?'), btn('Paste a code', () => pasteSheet('cahoots:ceremony:v1:…', take)), fileBtn('Load a file', take)),
    body: [next('Scan the host\'s code.'), h('p', { class: 'muted' }, 'For a ceremony with codes only, scan anyone who has joined. Their code is a short animation.'), fountainReceiver((bytes) => accept(parsePacked(bytes)), 'Fill the frame with the code', take).el, err],
    actions: [link('No camera? Paste a code', () => pasteSheet('cahoots:ceremony:v1:…', take))],
  });
}

/** Who you are, and where your randomness comes from. */
function screenPick(params) {
  // A joiner whose page reloaded has lost the host's code. It must not become a host.
  if (params.join && !pendingOffer) { ceremonyLost(); return; }
  // Joining through the relay: seat 1 is the host's.
  const joining = params.via === 'relay' && pendingOffer;
  const taken = new Set(joining ? [0] : pendingCommits.map((p) => p.index));
  let idx = [...Array(params.n).keys()].find((i) => !taken.has(i)) ?? 0;
  let src = 'csprng', extra = null;
  const err = status();
  const srcLine = h('span', { class: 'hint' }, 'Randomness: this device. The circle is labelled CSPRNG.');
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
      h('p', { class: 'muted' }, `Optional. A ${kib(params.pad_len)} file of random bytes from a hardware source is mixed in with this device's own randomness, never used alone. It earns this device the TRNG label.`),
      file, btn('Load a file', () => file.click()),
      h('p', { class: 'muted' }, 'Or scan an animation of the bytes shown on another screen:'), r.el,
      extra ? btn('Remove hardware randomness', () => { extra = null; src = 'csprng'; srcLine.textContent = 'Randomness: this device. The circle is labelled CSPRNG.'; close(); }, 'danger') : null);
  });
  screen({
    title: params.name, sub: `${params.n} people · ${kib(params.pad_len)}`, back: '#/',
    menu: hardwareSheet,
    body: [
      h('div', {},
        h('h1', {}, 'Which seat are you?'),
        h('p', { class: 'lead' }, joining ? 'Seat 1 is the host\'s. Pick a free one, and make sure nobody else picks the same.' : taken.size ? `Seat ${[...taken].map((i) => i + 1).join(', ')} is taken by the person you scanned. Pick a free one, and make sure nobody else picks the same.` : 'Each person takes a different seat. The person who set this up is seat 1.'),
        field('Your seat', seats),
        params.agent_seat ? notice('This circle invites a hosted software member. It will hold a copy of the secret in a secure enclave.') : null,
        srcLine, err),
    ],
    actions: [btn('Make my contribution', async () => {
      try {
        if (extra && extra.length !== params.pad_len) throw `The hardware file must be exactly ${params.pad_len} bytes, got ${extra.length}.`;
        const contribution = new Uint8Array(params.pad_len);
        for (let o = 0; o < contribution.length; o += 65536) crypto.getRandomValues(contribution.subarray(o, Math.min(o + 65536, contribution.length)));
        const kemSeed = crypto.getRandomValues(new Uint8Array(32));
        const seatSeed = new Uint8Array(32);
        let c = null;
        try {
          // A seat seed is a signing key; the rare draw that is not a valid one is drawn again.
          while (!c) {
            try {
              crypto.getRandomValues(seatSeed);
              c = joining
                ? Ceremony.from_offer(pendingOffer, idx, contribution, extra ?? undefined, kemSeed, seatSeed, PHONE_TIER)
                : new Ceremony(params.n, params.pad_len, params.name, params.agent_seat, idx, contribution, extra ?? undefined, kemSeed, seatSeed, PHONE_TIER);
            }
            catch (e) { if (e !== 'bad-seed') throw e; }
          }
        } finally {
          // The ceremony holds its own copies; wipe ours.
          kemSeed.fill(0); seatSeed.fill(0); contribution.fill(0);
        }
        const relay = params.via === 'relay';
        // The host opens the channel; its code is what everyone joining scans.
        const offer = relay ? c.offer(DEFAULT_RELAY) : null;
        pendingOffer = null;
        ceremony = { c, params, label: extra ? 'TRNG' : 'CSPRNG', mode: 'show', attests: [], revealShared: false, relay, offer, cursor: null, run: 0 };
        go(`#/ceremony/${paramsHash(params)}/run`);
      } catch (e) { err.textContent = friendly(e); }
    }, 'primary'), link('Add hardware randomness', hardwareSheet)],
  });
}

/** For a member still behind: our reveal (step 2) or our signature (step 3), as an animation and as text. */
function showRevealSheet(me, reveal) {
  const t = enc.reveal(me, reveal);
  sheet(h('h2', {}, 'Someone still on step 2? Show them this.'), animatedQr(packReveal(reveal), `seat ${me + 1} · step 2`), copyBtn(t, 'Copy my reveal'), downloadBtn(`reveal-${me + 1}.txt`, t));
}
function showSignSheet(me, sig, reveal) {
  const t = enc.attest(me, sig);
  sheet((close) => h('div', { class: 'stack' },
    h('h2', {}, 'Someone still on step 3? Show them this.'), animatedQr(packAttest(me, sig), `seat ${me + 1} · step 3`), copyBtn(t), downloadBtn(`sign-${me + 1}.txt`, t),
    link('They need my reveal instead', () => { close(); showRevealSheet(me, reveal); })));
}
const leave = () => { if (confirm('Leave the ceremony? Nothing is kept.')) { ceremony = null; go('#/'); } };
const STEPS = { commit: 'Step 1 of 4 · Meet the group', reveal: 'Step 2 of 4 · Exchange secrets', attest: 'Step 3 of 4 · Confirm members' };

/** Commit, reveal and sign: one stage with Show and Scan modes, member chips that update in place. */
function screenRun(params) {
  if (!ceremony) {
    ceremonyLost();
    return;
  }
  if (ceremony.relay) { screenRelay(params); return; }
  const { c } = ceremony;
  const me = c.index();
  for (const p of pendingCommits) { if (p.index !== me) { try { c.add_commitment(p.index, p.bytes); } catch (e) { /* shown as waiting */ } } }
  pendingCommits = [];
  // Signatures scanned before every reveal was in are kept until there is a list to check them against.
  if (c.phase() === 'attest') {
    for (const p of ceremony.attests) { try { c.add_attestation(p.index, p.bytes); } catch (e) { /* shown as waiting */ } }
    ceremony.attests = [];
  }
  const corePhase = c.phase();
  if (corePhase === 'aborted') {
    ceremony = null;
    screen({ title: 'Ceremony stopped', body: [h('div', {}, h('h1', {}, 'A reveal did not match its commitment.'), h('p', { class: 'lead' }, 'Nothing was kept. Either someone\'s contribution changed after they committed, or a code was corrupted. Start again from the beginning.'))], actions: [btn('Back to start', () => go('#/'), 'primary')] });
    return;
  }
  // Receiving everyone else's reveal says nothing about who has scanned ours.
  // Hold the reveal screen until the user confirms the outbound handoff too.
  const revealHandoff = !ceremony.revealShared && ['attest', 'complete'].includes(corePhase);
  const phase = revealHandoff ? 'reveal' : corePhase;
  if (revealHandoff) ceremony.mode = 'show';
  if (phase === 'complete') { screenSignDone(params); return; }
  const rerender = () => screenRun(params);
  const st = status();
  const seatName = (i) => (i === me ? 'you' : `seat ${i + 1}`);

  // Member chips and the "waiting for" line update in place, so the camera keeps running between scans.
  const chips = h('div', { class: 'members' });
  const taskTitle = h('strong', { class: 'ceremony-task' });
  const taskDetail = h('span');
  const waiting = h('div', { class: 'next ceremony-next' }, taskTitle, taskDetail);
  const received = h('p', { class: 'ceremony-progress', role: 'status', 'aria-live': 'polite' });
  const revealDone = btn(params.n > 2 ? 'Everyone has scanned my reveal' : 'They have scanned my reveal', () => {
    if (!['attest', 'complete'].includes(c.phase())) return;
    ceremony.revealShared = true;
    rerender();
  }, 'primary');
  const refresh = () => {
    if (c.phase() !== corePhase) { rerender(); return; }
    const missing = new Set(phase === 'commit' ? c.missing_commitments() : phase === 'reveal' ? c.missing_reveals() : c.missing_attestations());
    chips.replaceChildren(...Array.from({ length: params.n }, (_, i) =>
      h('span', { class: `chip${i === me ? ' me' : missing.has(i) ? '' : ' on'}` }, i === me ? `you · ${i + 1}` : `${i + 1}${missing.has(i) ? '' : ' ✓'}`)));
    const left = [...missing].filter((i) => i !== me);
    const noun = phase === 'commit' ? 'setup codes' : phase === 'reveal' ? 'reveals' : 'signatures';
    received.textContent = `Received ${params.n - 1 - left.length} of ${params.n - 1} ${noun}`;
    revealDone.disabled = !revealHandoff;
    revealDone.textContent = revealHandoff ? (params.n > 2 ? 'Everyone has scanned my reveal' : 'They have scanned my reveal') : 'Scan the remaining reveals to continue';
    if (!left.length) {
      taskTitle.textContent = phase === 'reveal' ? 'Your turn to show' : 'Ready for the next step';
      taskDetail.textContent = phase === 'reveal'
        ? 'Ask everyone to scan this code. Wait until they confirm before continuing.'
        : 'You have everything for this step.';
      return;
    }
    const who = left.map(seatName).join(', ');
    taskTitle.textContent = phase === 'commit' ? 'Meet the other devices' : phase === 'reveal' ? 'Collect their secret contributions' : 'Confirm who’s in the circle';
    taskDetail.textContent = phase === 'commit'
      ? `Scan the setup codes from ${who}. They also need to scan yours. You can take turns in any order.`
      : phase === 'reveal'
        ? `Ask ${who} to show their step-2 codes. Scan each animation until the progress bar fills.`
        : `Scan signatures from ${who} to confirm the group’s members. They also need to scan yours.`;
  };
  const got = (i, what) => { buzz(); toast(`Got seat ${i + 1}'s ${what}`, 'ok'); refresh(); };
  // A sealed reveal that arrives before its commitment stands in for it; the core handles that.
  const accept = (t) => {
    try {
      if (!t || t.kind === 'params') return;
      if (t.index === me) { st.textContent = `That is seat ${me + 1}'s, which is yours. If someone else chose seat ${me + 1} too, one of you must leave and rejoin with another seat.`; return; }
      if (t.kind === 'commit') { c.add_commitment(t.index, t.bytes); got(t.index, 'commitment'); }
      else if (t.kind === 'reveal') { c.add_reveal(t.index, t.bytes); got(t.index, 'reveal'); }
      else if (t.kind === 'attest') {
        if (c.phase() !== 'attest') {
          // Someone a step ahead. Keep the signature; it is checked once every reveal is in.
          ceremony.attests = ceremony.attests.filter((p) => p.index !== t.index).concat({ index: t.index, bytes: new Uint8Array(t.bytes) });
          buzz();
          st.textContent = `Seat ${t.index + 1} is already on step 3. Their signature is saved.${c.missing_reveals().includes(t.index) ? ' You still need their step-2 reveal. Ask them to select “Someone still needs my reveal”.' : ' Keep collecting the reveals listed above.'}`;
          return;
        }
        c.add_attestation(t.index, t.bytes); got(t.index, 'signature');
      }
    } catch (e) { st.textContent = friendly(e); if (c.phase() === 'aborted') rerender(); }
  };
  const feedText = (text) => accept(parseTagged(text));
  const feedBytes = (bytes) => accept(parsePacked(bytes));

  let show, text, fileName, reveal = null;
  // One camera for every step: all three codes are animations, and a member who is ahead shows
  // the next step's code while this one is still collecting.
  const scanStage = () => fountainReceiver(feedBytes, 'Fill the frame with their animation', feedText).el;
  if (phase === 'commit') {
    text = enc.commit(me, c.commitment(), c.params_bytes());
    fileName = `commit-${me + 1}.txt`;
    show = animatedQr(packCommit(me, c.params_bytes(), c.commitment()), `seat ${me + 1} · step 1`);
  } else if (phase === 'reveal') {
    reveal = revealOf(c);
    text = enc.reveal(me, reveal);
    fileName = `reveal-${me + 1}.txt`;
    show = animatedQr(packReveal(reveal), `seat ${me + 1} · step 2`);
  } else {
    reveal = revealOf(c); // others may still need it
    const sig = c.attestation();
    text = enc.attest(me, sig);
    fileName = `sign-${me + 1}.txt`;
    show = h('div', { class: 'stack' }, animatedQr(packAttest(me, sig), `seat ${me + 1} · step 3`), link('Someone still needs my reveal', () => showRevealSheet(me, reveal)));
  }
  const revealText = phase === 'attest' ? enc.reveal(me, reveal) : null;
  const desktop = matchMedia('(min-width: 960px)').matches;
  const showWrap = h('div', { class: 'half', style: desktop || ceremony.mode === 'show' ? '' : 'display:none' }, h('p', { class: 'eyebrow' }, 'Show'), show);
  const scanWrap = h('div', { class: 'half', style: desktop || ceremony.mode === 'scan' ? '' : 'display:none' }, h('p', { class: 'eyebrow' }, 'Scan'), desktop || ceremony.mode === 'scan' ? scanStage() : null);
  const modeSeg = seg([['show', phase === 'reveal' ? 'Show my reveal' : 'Show my code'], ['scan', phase === 'reveal' ? 'Scan their reveals' : 'Scan their codes']], ceremony.mode, (v) => {
    ceremony.mode = v;
    showWrap.style.display = v === 'show' ? '' : 'none';
    scanWrap.style.display = v === 'scan' ? '' : 'none';
    if (v !== 'scan') stopScanner(); else scanWrap.replaceChildren(h('p', { class: 'eyebrow' }, 'Scan'), scanStage());
  });
  if (desktop) modeSeg.classList.add('desktop-hide');
  const exchangeScreen = screen({
    title: STEPS[phase], sub: `${params.name} · you are seat ${me + 1}`,
    back: leave,
    menu: () => sheet(
      h('h2', {}, 'Share yours another way'),
      copyBtn(text), downloadBtn(fileName, text),
      revealText ? [h('h2', {}, 'Your reveal, for anyone still on step 2'), copyBtn(revealText, 'Copy my reveal'), downloadBtn(`reveal-${me + 1}.txt`, revealText)] : null,
      h('h2', {}, 'Receive theirs another way'),
      btn('Paste a code', () => pasteSheet(`cahoots-${phase}:…`, feedText)),
      fileBtn('Load a file', feedText),
      btn('Leave the ceremony', leave, 'danger')),
    body: [h('div', { class: 'ceremony-members' }, chips, received), waiting, modeSeg, h('div', { class: 'pair' }, showWrap, scanWrap), st],
    actions: phase === 'reveal' ? [revealDone] : [],
  });
  exchangeScreen.classList.add('ceremony-screen');
  refresh();
}

/** A ceremony through the relay. The host shows one code. Everyone's commitment, reveal and signature
 * go through the relay, sealed under that code's key, and this screen reads the relay every second. */
function screenRelay(params) {
  const cer = ceremony;
  const { c } = cer;
  const me = c.index();
  const run = ++cer.run;
  const live = () => ceremony === cer && cer.run === run && location.hash.endsWith('/run');
  // The host's relay, from the code: everyone in the ceremony uses it.
  const base = () => `${c.relay().replace(/\/$/, '')}/v1/circles/${c.channel_hex()}/messages`;
  const st = status();
  const chips = h('div', { class: 'members' });
  const received = h('p', { class: 'ceremony-progress', role: 'status', 'aria-live': 'polite' });
  const step = h('p', { class: 'next' });
  const stop = () => { cer.run++; };
  const leaveRelay = () => { if (confirm('Leave the ceremony? Nothing is kept.')) { stop(); ceremony = null; go('#/'); } };
  const host = me === 0;
  // What decides the layout. A change in it draws the screen again.
  const shape = () => `${c.phase()}|${c.needs_continue()}|${!!c.join_code()}`;
  // The host scans each joiner's code. Only a commitment that matches a scan goes into the host's ceremony.
  const scanned = h('ul', { class: 'scan-list' });
  const takeJoin = (text) => {
    try { const seat = c.scan_join(text.trim()); buzz(); toast(`Scanned seat ${seat + 1}`, 'ok'); st.textContent = ''; refresh(); }
    catch (e) { st.textContent = friendly(e); }
  };
  const scanSheet = () => sheet((close) => h('div', { class: 'stack' },
    h('h2', {}, 'Scan a joiner\'s code'),
    camera((text) => { if (text.startsWith(JOINED_PREFIX)) { stopScanner(); takeJoin(text); close(); } }, 'Fill the frame with their code'),
    btn('Paste a join code', () => { close(); pasteSheet('cahoots:joined:v1:…', takeJoin); })));
  const refresh = () => {
    const phase = c.phase();
    const missing = new Set(phase === 'commit' ? c.missing_commitments() : phase === 'reveal' ? c.missing_reveals() : phase === 'attest' ? c.missing_attestations() : []);
    const joined = phase === 'commit' ? params.n - c.missing_commitments().length : params.n;
    chips.replaceChildren(...Array.from({ length: params.n }, (_, i) =>
      h('span', { class: `chip${i === me ? ' me' : (phase === 'commit' ? missing.has(i) : false) ? '' : ' on'}` }, i === me ? `you · ${i + 1}` : `${i + 1}${phase === 'commit' && missing.has(i) ? '' : ' ✓'}`)));
    received.textContent = phase === 'commit' ? `${joined} of ${params.n} here` : `Everyone is here · ${params.n} of ${params.n}`;
    if (host && phase === 'commit') {
      const ok = new Set(c.scanned()), seen = new Set(c.unscanned());
      scanned.replaceChildren(...Array.from({ length: params.n - 1 }, (_, k) => k + 1).map((i) =>
        h('li', {}, ok.has(i) ? `Seat ${i + 1} · scanned ✓` : seen.has(i) ? `Seat ${i + 1} committed but was not scanned here.` : `Seat ${i + 1} · waiting`)));
    }
    step.textContent = c.needs_continue() ? 'Everyone is scanned.'
      : phase === 'commit'
        ? (host ? 'Show this code to everyone joining. Then scan the code on each of their screens.' : (c.join_code() ? 'Show this to the host.' : 'Wait here while everyone joins.'))
        : phase === 'reveal' ? (c.join_code() ? 'Show this to the host.' : 'Exchanging secrets through the relay.') : 'Confirming who is in the circle.';
  };
  const body = [h('div', { class: 'ceremony-members' }, chips, received), step];
  const join = c.join_code();
  if (host && c.phase() === 'commit') body.push(h('div', { class: 'pair' }, h('div', { class: 'half' }, panelQr(cer.offer, params.name, 'join'))), scanned);
  if (join) body.push(h('div', { class: 'pair' }, h('div', { class: 'half' }, panelQr(join, `seat ${me + 1}`, 'for the host'))));
  body.push(st);
  const actions = host && c.phase() === 'commit' ? [btn('Scan a joiner\'s code', scanSheet, 'primary')]
    : c.needs_continue() ? [btn('Continue', () => { c.proceed(); screenRelay(params); }, 'primary')] : [];
  screen({
    title: c.needs_continue() ? 'Everyone is scanned' : host && c.phase() === 'commit' ? 'Show this code to everyone joining' : join ? 'Show this to the host' : 'Through the relay', sub: `${params.name} · you are seat ${me + 1}`,
    back: leaveRelay,
    menu: () => sheet(...(host
      ? [h('h2', {}, 'The code to join'), panelQr(cer.offer, params.name, 'join'), copyBtn(cer.offer, 'Copy the code'), btn('Paste a join code', () => pasteSheet('cahoots:joined:v1:…', takeJoin))]
      : join ? [h('h2', {}, 'Your code for the host'), copyBtn(join, 'Copy my join code')] : []), btn('Leave the ceremony', leaveRelay, 'danger')),
    body,
    actions,
  }).classList.add('ceremony-screen');
  refresh();
  const shown = shape();
  const post = async () => {
    // In order, and stop at the first that fails: the rest go on the next tick.
    for (const w of c.outbound()) {
      const res = await fetch(base(), { method: 'POST', headers: { 'Content-Type': 'application/octet-stream' }, body: b64_decode(w) });
      if (res.status !== 201) throw new Error(`relay ${res.status}`);
      c.posted(w);
    }
  };
  const tick = async () => {
    if (!live()) return;
    try {
      await post();
      for (;;) {
        const url = `${base()}?wait=0${cer.cursor == null ? '' : `&after=${cer.cursor}`}`;
        const res = await fetch(url);
        if (!res.ok) throw new Error(`relay ${res.status}`);
        const { entries = [] } = await res.json();
        const fresh = entries.filter((e) => cer.cursor == null || e.seq > cer.cursor);
        if (!fresh.length) break;
        for (const e of fresh) {
          cer.cursor = e.seq;
          try { if (c.inbound(b64_decode(e.message))) buzz(); } catch (err) { st.textContent = friendly(err); }
        }
      }
      await post();
      if (st.textContent.startsWith('Cannot reach')) st.textContent = '';
    } catch (e) {
      if (!live()) return;
      st.textContent = 'Cannot reach the relay. Trying again.';
    }
    if (!live()) return;
    // Someone else's setup for this seat reached the relay first. This device posts nothing more.
    if (c.lost_seat()) {
      stop(); ceremony = null;
      screen({ title: 'Seat taken', body: [h('div', {}, h('h1', {}, `Seat ${me + 1} was taken by someone else.`), h('p', { class: 'lead' }, 'Leave and join again with a free seat.'))], actions: [btn('Leave', () => go('#/'), 'primary')] });
      return;
    }
    const phase = c.phase();
    if (phase === 'aborted') {
      stop(); ceremony = null;
      screen({ title: 'Ceremony stopped', body: [h('div', {}, h('h1', {}, 'A reveal did not match its commitment.'), h('p', { class: 'lead' }, 'Nothing was kept. Someone outside the room may have the code. Everyone starts over with a new code.'))], actions: [btn('Back to start', () => go('#/'), 'primary')] });
      return;
    }
    // Every signature is in, and ours is on the relay for the others.
    if (phase === 'complete' && !c.outbound().length) { stop(); screenWords(); return; }
    if (shape() !== shown) { screenRelay(params); return; }
    refresh();
    setTimeout(tick, CEREMONY_POLL_MS);
  };
  tick();
}

/** Every signature is in on this screen, but the others may still need this member's. Show it until they say so. */
function screenSignDone(params) {
  const { c } = ceremony;
  const me = c.index();
  const sig = c.attestation();
  const reveal = revealOf(c);
  const text = enc.attest(me, sig);
  const revealText = enc.reveal(me, reveal);
  const others = Array.from({ length: params.n }, (_, i) => i).filter((i) => i !== me).map((i) => `seat ${i + 1}`).join(' and ');
  screen({
    title: STEPS.attest, sub: params.name,
    back: leave,
    menu: () => sheet(
      h('h2', {}, 'Share your signature another way'), copyBtn(text), downloadBtn(`sign-${me + 1}.txt`, text),
      h('h2', {}, 'Your reveal, for anyone still on step 2'), copyBtn(revealText, 'Copy my reveal'), downloadBtn(`reveal-${me + 1}.txt`, revealText)),
    body: [
      next(`You have every signature. Before comparing words, make sure ${others} ${params.n > 2 ? 'have' : 'has'} scanned yours too. Keep this code on screen until they confirm.`),
      animatedQr(packAttest(me, sig), `seat ${me + 1} · step 3`),
      link('Someone still needs my reveal', () => showRevealSheet(me, reveal)),
    ],
    actions: [btn(params.n > 2 ? 'Everyone has scanned it' : 'They have scanned it', () => screenWords(), 'primary')],
  });
}

function screenWords() {
  const { c, label, params, relay } = ceremony;
  const me = c.index();
  // Kept so members still collecting can scan them from here.
  const reveal = revealOf(c);
  const sig = c.attestation();
  const ceremonyRelay = relay ? c.relay() : null;
  const out = c.finish(label);
  ceremony = null;
  const words = out.words().split(' ');
  const id = out.circle_id_hex();
  const showSign = () => showSignSheet(me, sig, reveal);
  screen({
    title: 'Step 4 of 4 · Compare words', sub: params.name,
    menu: relay ? null : showSign,
    body: [
      next('Read all six words aloud together. Check your own screen: every word must match, in the same order. If one differs, everyone starts again.'),
      h('div', { class: 'panel' },
        h('div', { class: 'cap' }, h('span', {}, params.name), h('span', {}, `${label} · ${id.slice(0, 8)}`)),
        h('div', { class: 'words' }, ...words.map((w, i) => h('div', {}, h('small', {}, String(i + 1)), w)))),
    ],
    actions: [
      btn('They match', async () => {
        const circle = out.into_circle(); live.set(id, circle); await saveCircle(circle);
        // The circle chats through the relay its ceremony used.
        if (ceremonyRelay && ceremonyRelay !== DEFAULT_RELAY) await saveSettings({ ...(await loadSettings(id)), id, relay: ceremonyRelay });
        toast('Your circle is ready.', 'ok'); go(`#/c/${id}`); }, 'primary'),
      btn('They differ · start over', () => { if (confirm('Throw this away and start again?')) go('#/'); }, 'danger'),
      relay ? null : link('Someone still needs my signature', showSign),
    ],
  });
}

// ---------------------------------------------------------------- a circle: hub, send, read, history, settings
const budget = (circle) => {
  const pct = Math.max(0, Math.min(100, Math.round((circle.remaining() / circle.slice_len()) * 100)));
  return h('div', { class: 'budget' }, h('div', { class: 'budget-bar' }, h('div', { style: `width:${pct}%` })), h('span', {}, `${kib(circle.remaining())} of your share left · ${pct}%`));
};
async function screenCircle(id) {
  return screenChat(id);
}

async function screenPeople(id) {
  const generation = routeGeneration;
  const circle = await openCircle(id);
  if (!circle) return go('#/');
  const settings = await loadSettings(id);
  const me = circle.my_slice();
  const rename = seat => sheet(close => {
    const name = input({ type: 'text', value: settings.names?.[seat] || '', maxlength: '24', autocomplete: 'off', 'aria-label': 'Name on this device', placeholder: `Seat ${seat + 1}` });
    return h('div', { class: 'stack' }, h('h2', {}, `A name for seat ${seat + 1}`), h('p', { class: 'muted' }, 'This name is just for you. It stays on this device and doesn’t verify anyone’s identity.'), field('Name on this device', name),
      btn('Save name', async () => {
        const names = { ...(settings.names || {}) };
        if (name.value.trim()) names[seat] = name.value.trim(); else delete names[seat];
        await saveSettings({ ...settings, id, names });
        close(); if (generation === routeGeneration) await screenPeople(id);
      }, 'primary'));
  });
  await workspaceScreen(circle, settings, { view: 'people', title: 'People', isCurrent: () => generation === routeGeneration,
    content: [h('header', { class: 'wallet-heading' }, h('p', { class: 'eyebrow' }, circle.name()), h('h1', {}, 'The people in this circle.'), h('p', { class: 'lead' }, 'Put a familiar name to each seat. Names are saved on your device only.')),
      h('div', { class: 'people-grid' }, ...Array.from({ length: circle.members() }, (_, seat) => h('article', { class: 'person-card' },
        circleMark(id.slice(seat * 2) || id, settings.names?.[seat] || String(seat + 1)),
        h('div', {}, h('h2', {}, seat === me ? 'You' : nameOf(settings, seat)), h('p', {}, `Seat ${seat + 1}${seat === me ? ' · this device' : ''}`)),
        seat === me ? h('span', { class: 'person-local' }, 'Your place in the circle') : h('div', { class: 'person-actions' }, btn(settings.names?.[seat] ? 'Edit name' : 'Name this person', () => rename(seat)), h('a', { href: `#/c/${id}/hello/${seat}` }, 'Compare hello words ↗'))))),
      h('p', { class: 'device-note' }, 'Seat labels identify devices in the ceremony. A saved name or decorative mark is not proof of identity.')],
  });
}

async function screenSend(id) {
  const generation = routeGeneration;
  const circle = await openCircle(id);
  if (!circle) return go('#/');
  let type = 0, padOn = true;
  const text = h('textarea', { class: 'input text', placeholder: 'Up to 255 characters', maxlength: '255', rows: 4 });
  const count = h('span', { class: 'hint' }, '0 / 255');
  text.oninput = () => { count.textContent = `${utf8.encode(text.value).length} / 255`; };
  const err = status();
  if (generation !== routeGeneration) return;
  screen({
    title: 'Send a message', sub: `${kib(circle.remaining())} of your share left`, back: `#/c/${id}`,
    menu: () => sheet(
      h('h2', {}, 'Message options'),
      field('Kind', seg(TYPES.map((t, i) => [i, t]), type, (v) => { type = Number(v); }), 'Text is the normal kind. The others are for programs that read the feed.'),
      h('label', { class: 'toggle' }, h('span', {}, h('b', {}, 'Hide the length'), h('br'), h('span', { class: 'muted' }, 'Pads to 32, 64, 128, or 256 bytes so the size gives nothing away.')), h('input', { type: 'checkbox', checked: padOn, onchange: (e) => { padOn = e.target.checked; } }))),
    body: [
      h('div', {},
        h('h1', {}, 'Write it.'),
        h('p', { class: 'lead' }, 'Encrypt a short note with this circle’s shared secret, then pass it along as a code or text.'),
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
    body: [next('Let them scan this, or send it as text. Post it anywhere: only your circle can read it.'), panelQr(wire, circle.name(), 'scan with Read a message')],
    actions: [btn('Done', () => go(`#/c/${id}`), 'primary'), link('Copy as text', async () => { await navigator.clipboard.writeText(wire); toast('Copied'); })],
  });
}
async function screenRead(id) {
  const generation = routeGeneration;
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
      if (generation !== routeGeneration) return;
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
  const generation = routeGeneration;
  const circle = await openCircle(id);
  if (!circle) return go('#/');
  const msgs = (await loadMessages(id)).reverse();
  if (generation !== routeGeneration) return;
  screen({
    title: 'History', sub: circle.name(), back: `#/c/${id}`,
    body: [msgs.length
      ? h('div', { class: 'top' }, ...msgs.map((m) => h('div', { class: 'msg' }, h('div', { class: 'meta' }, `${m.dir === 'out' ? 'you sent' : `from seat ${m.slice + 1}`}${m.type !== 'text' ? ` · ${m.type}` : ''} · ${new Date(m.ts).toLocaleString()}`), h('div', { class: 'plain' }, m.text))))
      : h('div', { class: 'empty' }, h('h1', {}, 'Nothing yet.'), h('p', { class: 'lead' }, 'Messages you send or read show up here.'))],
  });
}

// ---------------------------------------------------------------- hello words: pairwise, spoken, moved on demand
async function screenHello(id, peerStr) {
  const generation = routeGeneration;
  const circle = await openCircle(id);
  if (!circle) return go('#/');
  const me = circle.my_slice();
  const settings = await loadSettings(id);
  if (generation !== routeGeneration) return;
  const others = [...Array(circle.members()).keys()].filter((s) => s !== me);
  const tooOld = () => screen({
    title: 'Hello words', sub: circle.name(), back: `#/c/${id}`,
    body: [h('div', { class: 'empty' }, h('h1', {}, 'This circle is from before hello words.'), h('p', { class: 'lead' }, 'The words come from keys made during the ceremony. Run a new ceremony and they will be there.'))],
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

// ---------------------------------------------------------------- chat (root epochs, over the Numbers Station)
const pendingChatSends = new Map(); // One in-flight send per circle, across remounted views.
const chatDrafts = new Map(); // Unsent text stays in memory, scoped to its circle.
let chatRun = 0; // bumps on every render; a stale poll loop sees a different value and stops
window.addEventListener('hashchange', () => { chatRun++; });
async function screenChat(id) {
  const generation = routeGeneration;
  const requestedHash = location.hash;
  const circle = await openCircle(id);
  if (!circle) return go('#/');
  const me = circle.my_slice();
  const settings = await loadSettings(id);
  if (location.hash !== requestedHash || generation !== routeGeneration) return;
  const run = ++chatRun;
  const relayBase = () => (settings.relay || '').trim().replace(/\/$/, '');
  const st = status();
  const list = h('div', { class: 'chatlist' });
  const inp = input({ type: 'text', placeholder: 'Message', value: chatDrafts.get(id) || '', maxlength: '4000', autocomplete: 'off', enterkeyhint: 'send', oninput: e => { if (e.target.value) chatDrafts.set(id, e.target.value); else chatDrafts.delete(id); } });
  const pending = pendingChatSends.get(id);
  const sendBtn = btn(pending ? 'Sending…' : 'Send', () => doSend(), 'primary');
  inp.disabled = sendBtn.disabled = !!pending;
  if (pending) pending.refresh = async error => { if (run === chatRun) { await screenChat(id); if (error) toast(error, 'bad'); } };
  const epoch = () => { const e = circle.current_epoch(); return e ? `epoch ${e}` : 'no epoch yet'; };
  const bubble = (m) => m.dir === 'rejected'
    ? h('div', { class: 'bubble rejected' }, `rejected · ${friendly(m.reason)} · seq ${m.seq}`)
    : h('div', { class: `bubble ${m.dir}` }, h('div', { class: 'meta' }, `${m.dir === 'out' ? 'you' : nameOf(settings, m.sender)} · ${new Date(m.ts).toLocaleTimeString()}`), h('div', {}, m.text));
  const empty = h('div', { class: 'chat-empty' }, h('p', { class: 'eyebrow' }, 'A little space for your people'), h('h1', {}, 'Keep the conversation going.'), h('p', {}, 'Start with a hello. Messages are encrypted on your device before the relay delivers them.'));
  const append = (m) => { empty.remove(); list.append(bubble(m)); list.scrollTop = list.scrollHeight; };
  const history = await loadChat(id);
  if (run !== chatRun) return;
  for (const m of history) append(m);
  const titleSub = h('small', {}, `${circle.members()} people · End-to-end encrypted`);

  const relaySheet = () => sheet((close) => {
    const relayIn = input({ type: 'url', value: settings.relay, placeholder: 'https://relay.example.com', autocapitalize: 'off', autocomplete: 'off' });
    return h('div', { class: 'stack' },
      h('h2', {}, 'Relay'),
      h('p', { class: 'muted' }, `Messages travel through a relay that every member can reach. It only ever sees encrypted messages. The app ships with ${DEFAULT_RELAY.replace('https://', '')}. Every member of this circle must use the same relay, so change it together.`),
      field('Relay address', relayIn),
      btn('Save and connect', async () => { settings.relay = relayIn.value.trim() || DEFAULT_RELAY; await saveSettings({ ...settings, id }); close(); chatRun++; screenChat(id); }, 'primary'),
      settings.relay !== DEFAULT_RELAY ? btn('Use the default relay', async () => { settings.relay = DEFAULT_RELAY; await saveSettings({ ...settings, id }); close(); chatRun++; screenChat(id); }) : null);
  });

  const post = async (bytes) => {
    const res = await fetch(`${relayBase()}/v1/circles/${id}/messages`, { method: 'POST', headers: { 'Content-Type': 'application/octet-stream' }, body: bytes });
    if (!res.ok) throw `relay ${res.status}: ${await res.text()}`;
    return res.json();
  };
  const doSend = async () => {
    const text = inp.value;
    if (!text || pendingChatSends.has(id)) return;
    if (!relayBase()) { relaySheet(); return; }
    const sending = { refresh: null };
    pendingChatSends.set(id, sending);
    let sendError = null;
    sendBtn.disabled = true;
    sendBtn.textContent = 'Sending…';
    inp.disabled = true;
    try {
      const pt = chat_bucket_pad(utf8.encode(text));
      if (!pt) throw 'Message too long.';
      // The first send after a load waits until the relay has been read to the end, so this
      // device knows where its own messages stopped.
      for (let i = 0; i < 100 && !circle.chat_synced() && run === chatRun; i++) await new Promise((r) => setTimeout(r, 100));
      const wire = circle.chat_send(pt);
      await saveCircle(circle); // the chain step is recorded before the ciphertext leaves
      await post(b64_decode(wire));
      const m = { circle: id, dir: 'out', sender: me, epoch: circle.current_epoch(), text, ts: Date.now() };
      await saveChat(m); append(m); titleSub.textContent = `${circle.members()} people · End-to-end encrypted`;
      if (chatDrafts.get(id) === text) chatDrafts.delete(id);
      inp.value = '';
    } catch (e) { sendError = `Not sent: ${friendly(e)}`; st.textContent = sendError; }
    finally { pendingChatSends.delete(id); }
    sendBtn.disabled = false;
    sendBtn.textContent = 'Send';
    inp.disabled = false;
    if (run === chatRun) inp.focus();
    else if (sending.refresh) await sending.refresh(sendError);
  };
  inp.onkeydown = (e) => { if (e.key === 'Enter') doSend(); };

  const poll = async () => {
    while (run === chatRun) {
      const after = circle.relay_seq();
      // Until the relay has been read to the end, poll without waiting.
      const wait = circle.chat_synced() || !circle.has_root() ? POLL_WAIT_MS : 0;
      const url = `${relayBase()}/v1/circles/${id}/messages?wait=${wait}${after == null ? '' : `&after=${after}`}`;
      let data;
      try {
        const res = await fetch(url);
        if (!res.ok) throw `${res.status}`;
        data = await res.json();
        st.textContent = 'Connected to relay';
      } catch (e) {
        st.textContent = `Cannot reach ${relayBase().replace('https://', '')}. Retrying. If this keeps up, check the address in the menu.`;
        await new Promise((r) => setTimeout(r, 3000));
        continue;
      }
      if (run !== chatRun) break;
      let changed = false;
      const entries = data.entries || [];
      if (!entries.length && !circle.chat_synced()) circle.chat_mark_synced();
      for (const entry of entries) {
        const bytes = b64_decode(entry.message);
        // Our own messages go through chat_receive too: it moves our sending position past
        // them without decrypting, so a device restored from older state never reuses a key.
        if (bytes[0] === 2 && bytes[22] === me) {
          try { circle.chat_receive(entry.message); } catch (e) { /* a forged or damaged copy of our own: nothing to show */ }
        } else if (bytes[0] === 2) {
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
      if (changed) { await saveCircle(circle); titleSub.textContent = `${circle.members()} people · End-to-end encrypted`; }
    }
  };

  const noRelay = h('div', { class: 'empty' },
    h('h1', {}, 'Chat needs a relay.'),
    h('p', { class: 'lead' }, 'A relay is a small server any member can run. It carries encrypted messages between you and cannot read them. Set its address to start.'),
    btn('Set the relay address', relaySheet, 'primary'));
  const conversation = h('div', { class: 'dashboard-conversation' },
    h('header', { class: 'conversation-heading' }, h('div', {}, h('h1', {}, 'Conversation'), titleSub),
      btn('Circle tools', () => workspace.tools(), 'dashboard-tools-trigger')),
    h('div', { class: 'conversation-thread' }, ...(relayBase() ? [history.length ? null : empty, list] : [noRelay])),
    st,
    !relayBase() ? null
      : circle.has_root() ? h('div', { class: 'dashboard-composer composer' }, inp, sendBtn)
      : h('p', { class: 'muted' }, 'This circle was made before root epochs. Run a new ceremony to chat.'));
  const aside = h('aside', { class: 'dashboard-aside' }, h('header', {}, h('h2', {}, 'People'), h('span', {}, String(circle.members()))),
    h('div', { class: 'dashboard-members' }, ...Array.from({ length: circle.members() }, (_, seat) => h('a', { href: `#/c/${id}/people` }, circleMark(id.slice(seat * 2) || id, String(seat + 1)), h('div', {}, h('b', {}, seat === me ? 'You' : nameOf(settings, seat)), h('span', {}, `Seat ${seat + 1}${seat === me ? ' · this device' : ''}`))))),
    h('a', { class: 'manage-people', href: `#/c/${id}/people` }, 'Manage names ↗'),
    h('div', { class: 'dashboard-context' }, h('h2', {}, 'About this circle'), h('p', {}, 'Created together in person. Your keys and history are stored in this browser.'), h('a', { href: `${SITE}/docs/#/security`, target: '_blank', rel: 'noopener' }, 'Security & limits ↗')));
  const workspace = await workspaceScreen(circle, settings, { content: [conversation], aside, isCurrent: () => run === chatRun,
    menu: () => sheet((close) => h('div', { class: 'stack' }, h('h2', {}, 'Conversation settings'), h('p', { class: 'eyebrow' }, CHAT_LABEL), h('p', { class: 'muted' }, CHAT_CLAIM),
      h('p', { class: 'wallet-fine' }, `Current keys: ${epoch()}`),
      h('p', { class: 'muted' }, 'A sender label identifies a message chain, not proof of which member typed it.'),
      circle.has_root() ? h('p', { class: 'muted' }, 'Starting a new epoch moves everyone to new keys from here on. It does not protect older messages.') : null,
      circle.has_root() ? btn('Start a new epoch', async () => { try { circle.rotate_epoch(); await saveCircle(circle); toast('New epoch started', 'ok'); close(); } catch (e) { st.textContent = friendly(e); } }) : null,
      btn('Relay address', () => { close(); relaySheet(); }))),
  });
  if (!workspace) return;
  inp.setAttribute('aria-label', 'Message');
  if (relayBase()) poll();
}

// ---------------------------------------------------------------- router
async function route() {
  routeGeneration++;
  const parts = location.hash.replace(/^#\/?/, '').split('/');
  try {
    if (parts[0] === 'new') screenNew();
    else if (parts[0] === 'join') screenJoin();
    else if (parts[0] === 'explore') go('#/');
    else if (parts[0] === 'ceremony') {
      const params = JSON.parse(utf8d.decode(b64_decode(parts[1])));
      if (parts[2] === 'run') screenRun(params); else screenPick(params);
    } else if (parts[0] === 'c') {
      if (parts[2] === 'send') await screenSend(parts[1]);
      else if (parts[2] === 'read') await screenRead(parts[1]);
      else if (parts[2] === 'history') await screenHistory(parts[1]);
      else if (parts[2] === 'network') await screenNetwork(parts[1]);
      else if (parts[2] === 'people') await screenPeople(parts[1]);
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
