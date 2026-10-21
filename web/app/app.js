// Cahoots phone/laptop client. The ceremony and pure-pad messaging make no network calls.
// The chat screen talks to a Numbers Station relay over HTTP; it sends and receives ciphertext only.
// All crypto is in cahoots-core and cahoots-chat (WASM).
//
// UI rule: one job per screen. A top bar, one stage element, one primary action.
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
  'wrong-circle': 'This message belongs to a different circle.',
  'out-of-range': 'The message points outside the pad.',
  exhausted: 'Your share of the pad is used up. Hold a new ceremony.',
  base64url: 'That is not a Cahoots message.',
  replay: 'Already received.',
  'too-far-ahead': 'Too many messages were skipped before this one.',
  version: 'Not a chat message.',
  length: 'Malformed message.',
};
const CHAT_LABEL = 'End-to-end encrypted, pad-hedged';
const CHAT_CLAIM = 'The pad seeds the keys and ChaCha20-Poly1305 encrypts. The relay sees ciphertext and traffic shape only.';
const DEFAULT_RELAY = 'http://127.0.0.1:8080';
const POLL_WAIT_MS = 25000;
const QR_PACKET = 300;   // payload bytes per animated frame: ~77 modules per side, readable by phone cameras
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

/** The screen skeleton: bar (back, title, menu), body with a stage, pinned actions. */
function screen({ title, sub, back, menu, body, actions = [] }) {
  stopScanner();
  const bar = h('div', { class: 'bar' },
    back ? h('button', { 'aria-label': 'Back', onclick: () => (typeof back === 'function' ? back() : go(back)) }, '‹') : h('span'),
    h('div', { class: 'title' }, title, sub ? h('small', {}, sub) : null),
    menu ? h('button', { 'aria-label': 'More', onclick: menu }, '⋯') : h('span'));
  const stage = h('div', { class: 'stage' }, ...body);
  const el = h('div', { class: 'screen' }, bar, h('div', { class: 'body' }, stage), actions.length ? h('div', { class: 'actions' }, ...actions) : null);
  app.replaceChildren(el);
  window.scrollTo(0, 0);
  return el;
}
const btn = (label, onclick, kind = '') => h('button', { class: `btn ${kind}`, onclick }, label);
const link = (label, onclick) => h('button', { class: 'link', onclick }, label);
const notice = (text, kind = '') => h('div', { class: `notice ${kind}` }, text);
const status = () => h('p', { class: 'status' }, '');
function seg(options, value, onChange) {
  const el = h('div', { class: 'seg' });
  const set = (v) => { value = v; for (const b of el.children) b.classList.toggle('on', b.dataset.v === String(v)); onChange(v); };
  for (const [v, label] of options) el.append(h('button', { 'data-v': v, class: String(v) === String(value) ? 'on' : '', onclick: () => set(v) }, label));
  return el;
}
function field(label, input) { return h('div', { class: 'field' }, h('label', {}, label), input); }
const input = (attrs) => h('input', { class: 'input', ...attrs });
function stepper(min, max, value, onChange) {
  const out = h('output', {}, String(value));
  const set = (v) => { value = Math.min(max, Math.max(min, v)); out.textContent = String(value); onChange(value); };
  return h('div', { class: 'stepper' }, h('button', { onclick: () => set(value - 1), 'aria-label': 'Fewer' }, '−'), out, h('button', { onclick: () => set(value + 1), 'aria-label': 'More' }, '+'));
}
/** A bottom sheet for the secondary options a screen needs but shouldn't show by default. */
function sheet(...kids) {
  const bg = h('div', { class: 'sheet-bg', onclick: (e) => { if (e.target === bg) bg.remove(); } });
  const close = () => bg.remove();
  bg.append(h('div', { class: 'sheet' }, h('div', { class: 'grab' }), ...kids.map((k) => (typeof k === 'function' ? k(close) : k))));
  document.body.append(bg);
  return close;
}
function panelQr(text, capLeft, capRight) {
  const box = h('div', { class: 'qr' });
  try { box.innerHTML = qr_svg(utf8.encode(text)); } catch (e) { box.replaceChildren(h('p', { class: 'muted' }, 'Too large for one code. Use the text form in the menu.')); }
  return h('div', { class: 'panel' }, h('div', { class: 'cap' }, h('span', {}, capLeft), h('span', {}, capRight)), box);
}
const copyBtn = (text, label = 'Copy text') => btn(label, async (e) => { await navigator.clipboard.writeText(text); e.target.textContent = 'Copied'; });
const downloadBtn = (name, text) => btn('Download file', () => {
  const a = h('a', { href: URL.createObjectURL(new Blob([text], { type: 'text/plain' })), download: name });
  a.click(); URL.revokeObjectURL(a.href);
});
function pasteSheet(placeholder, onText) {
  return sheet((close) => {
    const ta = h('textarea', { class: 'input', placeholder });
    return h('div', { style: 'display:grid;gap:10px' }, h('h2', {}, 'Paste a code'), ta, btn('Use this', () => { onText(ta.value); close(); }, 'primary'));
  });
}

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
    box.append(h('p', { class: 'muted', style: 'padding:20px;text-align:center' }, 'This browser cannot open the camera. Paste a code from the menu instead.'));
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
          if (frames % 10 === 0) hintEl.textContent = hits ? `Reading · ${hits} frames` : hint;
        } catch (e) { /* frame not ready */ }
        busy = false;
      }, 1000 / QR_FPS / 2);
      scanner = { stream, timer };
    } catch (e) {
      box.replaceChildren(h('p', { class: 'muted', style: 'padding:20px;text-align:center' }, `Camera unavailable: ${e.message}`), btn('Try again', start));
    }
  };
  if (cameraOk) start(); else box.append(btn('Open camera', start, 'primary'));
  return box;
}

// ---------------------------------------------------------------- animated QR (fountain)
function animatedQr(bytes, capLeft) {
  const enc = new FountainEncoder(bytes, QR_PACKET);
  const box = h('div', { class: 'qr' });
  const cap = h('span', {}, '');
  let i = 0;
  const tick = () => {
    box.innerHTML = qr_svg(utf8.encode(b64_encode(enc.packet(i))));
    cap.textContent = `frame ${i + 1} · ${enc.source_packets()} needed`;
    i += 1;
  };
  tick();
  const timer = setInterval(tick, 1000 / QR_FPS);
  const obs = new MutationObserver(() => { if (!document.body.contains(box)) { clearInterval(timer); obs.disconnect(); } });
  obs.observe(document.body, { childList: true, subtree: true });
  return h('div', { class: 'panel' }, h('div', { class: 'cap' }, h('span', {}, capLeft), cap), box);
}
/** Camera plus progress; calls onDone(bytes) when the transfer completes. */
function fountainReceiver(onDone, hint) {
  let dec = new FountainDecoder();
  const bar = h('progress', { value: 0, max: 1 });
  const st = status();
  st.textContent = 'Waiting for frames';
  const feed = (text) => {
    try {
      const done = dec.add(b64_decode(text));
      bar.value = dec.progress();
      st.textContent = `${Math.round(dec.progress() * 100)}% of ${kib(dec.transfer_length())}`;
      if (done) { const out = dec.result(); dec = new FountainDecoder(); st.textContent = 'Done'; onDone(out); }
    } catch (e) { if (e !== 'magic') st.textContent = `Ignored frame: ${e}`; }
  };
  return { el: h('div', { style: 'display:grid;gap:10px' }, camera(feed, hint), bar, st), feed };
}

// ---------------------------------------------------------------- text/file codecs
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
const packReveal = (i, bytes) => { const out = new Uint8Array(bytes.length + 1); out[0] = i; out.set(bytes, 1); return out; };
const paramsHash = (params) => b64_encode(utf8.encode(JSON.stringify(params)));
let pendingCommits = [];   // commitments scanned before this device's ceremony object exists
let pendingReveals = new Map(); // reveals that arrived before every commitment was in
let ceremony = null;       // in-memory only: { c, params, label }

// ================================================================ screens
async function screenHome() {
  const rows = await loadCircles();
  const list = rows.length
    ? h('div', { class: 'rows' }, ...rows.map((r) => h('a', { class: 'row-item', href: `#/c/${r.id}` },
        h('div', {}, h('b', {}, r.name), h('span', {}, `${r.id.slice(0, 8)} · `), h('span', { class: `pill ${r.label.toLowerCase()}` }, r.label)),
        h('span', { class: 'arrow' }, '›'))))
    : h('div', { style: 'text-align:center' }, h('h1', {}, 'No cahoots yet.'), h('p', { class: 'lead' }, 'Get the people in the room, and start one.'));
  screen({
    title: 'Cahoots',
    body: [rows.length ? h('div', {}, h('p', { class: 'eyebrow' }, 'Your cahoots'), list) : list],
    actions: [btn('Start a ceremony', () => go('#/new'), 'primary'), btn('Join a ceremony', () => go('#/join'))],
  });
}

function screenNew() {
  const params = { n: 2, pad_len: 65536, name: 'sf-meetup', agent_seat: false };
  const name = input({ type: 'text', value: params.name, maxlength: '64', oninput: (e) => { params.name = e.target.value; } });
  const agent = h('input', { type: 'checkbox', onchange: (e) => { params.agent_seat = e.target.checked; } });
  screen({
    title: 'New ceremony', back: '#/',
    body: [
      h('div', {},
        h('h1', {}, 'Set up the cahoot.'),
        h('p', { class: 'lead' }, 'Everyone will see these on their own screen and confirm.'),
        field('Name', name),
        field('People in the room', stepper(2, 8, params.n, (v) => { params.n = v; })),
        field('Pad size', seg([[16384, '16 KiB'], [65536, '64 KiB'], [262144, '256 KiB']], params.pad_len, (v) => { params.pad_len = v; })),
        h('p', { class: 'muted' }, '64 KiB is a few hundred messages and about a minute per person to exchange by camera.'),
        h('label', { class: 'toggle' }, h('span', {}, h('b', {}, 'Invite a hosted software member'), h('br'), h('span', { class: 'muted' }, 'Its secret lives in a secure enclave. Leave off for a people-only cahoot.')), agent)),
    ],
    actions: [btn('Continue', () => go(`#/ceremony/${paramsHash({ ...params, name: params.name.trim() || 'circle' })}/pick`), 'primary')],
  });
}

function screenJoin() {
  const err = status();
  const take = (text) => {
    try {
      const t = parseTagged(text);
      if (!t) throw 'That is not a Cahoots code.';
      if (t.kind === 'reveal') throw 'That is a reveal code. Scan a commitment code instead.';
      if (t.kind === 'commit' && !t.params) throw 'That commitment has no setup in it. Ask the leader to show their code.';
      const params = paramsFromBytes(t.kind === 'params' ? t.bytes : t.params);
      if (t.kind === 'commit') pendingCommits = [{ index: t.index, bytes: t.bytes }];
      go(`#/ceremony/${paramsHash(params)}/pick`);
    } catch (e) { err.textContent = String(e); }
  };
  screen({
    title: 'Join', sub: 'scan anyone who has joined', back: '#/',
    menu: () => pasteSheet('cahoots-commit:…', take),
    body: [camera(take, 'Point at another member\'s code'), err],
    actions: [link('Paste a code instead', () => pasteSheet('cahoots-commit:…', take))],
  });
}

/** Who you are, and where your randomness comes from. */
function screenPick(params) {
  let idx = 0, src = 'csprng', extra = null;
  const err = status();
  const pickIdx = seg(Array.from({ length: params.n }, (_, i) => [i, i === 0 ? 'Leader' : `${i + 1}`]), 0, (v) => { idx = Number(v); });
  const file = h('input', { type: 'file', style: 'display:none', onchange: async (e) => {
    if (!e.target.files[0]) return;
    extra = new Uint8Array(await e.target.files[0].arrayBuffer());
    err.textContent = `Loaded ${kib(extra.length)} from file.`;
  } });
  const srcChoice = h('div', { class: 'choice' },
    h('label', {}, h('input', { type: 'radio', name: 'src', value: 'csprng', checked: true, onchange: () => { src = 'csprng'; } }), h('div', {}, h('b', {}, 'This device'), h('span', {}, 'The phone\'s random generator. The cahoot is labelled CSPRNG.'))),
    h('label', {}, h('input', { type: 'radio', name: 'src', value: 'file', onchange: () => { src = 'file'; file.click(); } }), h('div', {}, h('b', {}, 'This device + a YubiKey file'), h('span', {}, `A ${kib(params.pad_len)} file from cahoots-yk. Mixed in, never used alone. Labelled TRNG.`))),
    h('label', {}, h('input', { type: 'radio', name: 'src', value: 'qr', onchange: () => { src = 'qr'; importSheet(); } }), h('div', {}, h('b', {}, 'This device + animated code from a laptop'), h('span', {}, 'Scan the laptop\'s animation. Mixed in, never used alone. Labelled TRNG.'))));
  const importSheet = () => sheet((close) => {
    const r = fountainReceiver((bytes) => { extra = bytes; err.textContent = `Imported ${kib(bytes.length)} by camera.`; close(); }, 'Point at the laptop\'s animation');
    return h('div', { style: 'display:grid;gap:10px' }, h('h2', {}, 'Import randomness'), r.el);
  });
  screen({
    title: params.name, sub: `${params.n} people · ${kib(params.pad_len)}`, back: '#/',
    body: [
      h('div', {},
        h('h1', {}, 'Who are you?'),
        h('p', { class: 'lead' }, 'Each person takes a different seat. The leader is seat 1.'),
        field('Your seat', pickIdx),
        field('Randomness', srcChoice), file,
        params.agent_seat ? notice('This cahoot invites a hosted software member. Its secret lives in a secure enclave.') : null,
        err),
    ],
    actions: [btn('Make my contribution', async () => {
      try {
        if (src !== 'csprng' && !extra) throw 'No hardware randomness loaded yet.';
        if (extra && extra.length !== params.pad_len) throw `The hardware file must be exactly ${params.pad_len} bytes, got ${extra.length}.`;
        const contribution = new Uint8Array(params.pad_len);
        for (let o = 0; o < contribution.length; o += 65536) crypto.getRandomValues(contribution.subarray(o, Math.min(o + 65536, contribution.length)));
        const c = new Ceremony(params.n, params.pad_len, params.name, params.agent_seat, idx, contribution, extra ?? undefined);
        ceremony = { c, params, label: extra ? 'TRNG' : 'CSPRNG', mode: 'show' };
        go(`#/ceremony/${paramsHash(params)}/run`);
      } catch (e) { err.textContent = String(e); }
    }, 'primary')],
  });
}

/** Commit and reveal: one stage with Show and Scan modes, member chips, auto-advance. */
function screenRun(params) {
  if (!ceremony) { go(`#/ceremony/${paramsHash(params)}/pick`); return; }
  const { c } = ceremony;
  const me = c.index();
  for (const p of pendingCommits) { if (p.index !== me) { try { c.add_commitment(p.index, p.bytes); } catch (e) { /* shown as waiting */ } } }
  pendingCommits = [];
  if (c.phase() === 'reveal') {
    for (const [i, bytes] of pendingReveals) { try { c.add_reveal(i, bytes); } catch (e) { /* re-shown as waiting */ } }
    pendingReveals = new Map();
  }
  const phase = c.phase();
  if (phase === 'aborted') {
    ceremony = null;
    screen({ title: 'Ceremony stopped', body: [h('div', {}, h('h1', {}, 'A reveal did not match its commitment.'), h('p', { class: 'lead' }, 'Nothing was kept. Someone\'s contribution changed after they committed, or a code was corrupted. Start again from the beginning.'))], actions: [btn('Back to start', () => go('#/'), 'primary')] });
    return;
  }
  if (phase === 'complete') { screenWords(); return; }
  const refresh = () => screenRun(params);
  const st = status();
  const missing = new Set(phase === 'commit' ? c.missing_commitments() : c.missing_reveals());
  const chips = h('div', { class: 'members' }, ...Array.from({ length: params.n }, (_, i) =>
    h('span', { class: `chip${i === me ? ' me' : missing.has(i) ? '' : ' on'}` }, i === me ? 'you' : `${i + 1} ${missing.has(i) ? '' : '✓'}`)));
  const feedText = (text) => {
    try {
      const t = parseTagged(text);
      if (!t || t.kind === 'params' || t.index === me) return;
      if (t.kind === 'commit') c.add_commitment(t.index, t.bytes);
      else if (t.kind === 'reveal') acceptReveal(t.index, t.bytes);
      refresh();
    } catch (e) { st.textContent = String(e); if (c.phase() === 'aborted') refresh(); }
  };
  // A reveal fixes its commitment, so a member that has not revealed yet may take a late
  // reveal as both. The reveal itself is applied once every commitment is in.
  const acceptReveal = (i, bytes) => {
    if (c.phase() === 'commit') { c.add_commitment(i, c.commitment_for(i, bytes)); pendingReveals.set(i, bytes); }
    else c.add_reveal(i, bytes);
  };
  const feedBytes = (bytes) => {
    try { if (bytes[0] === me) return; acceptReveal(bytes[0], bytes.subarray(1)); refresh(); }
    catch (e) { st.textContent = String(e); if (c.phase() === 'aborted') refresh(); }
  };

  let show, scan, text, fileName;
  if (phase === 'commit') {
    text = enc.commit(me, c.commitment(), c.params_bytes());
    fileName = `commit-${me}.txt`;
    show = panelQr(text, `seat ${me + 1} · commitment`, 'others scan this');
    scan = h('div', {}, camera(feedText, 'Point at another member\'s code'));
  } else {
    const reveal = c.reveal();
    text = enc.reveal(me, reveal);
    fileName = `reveal-${me}.txt`;
    show = animatedQr(packReveal(me, reveal), `seat ${me + 1} · reveal`);
    scan = fountainReceiver(feedBytes, 'Point at another member\'s animation').el;
  }
  const desktop = matchMedia('(min-width: 960px)').matches;
  const showWrap = h('div', { style: desktop || ceremony.mode === 'show' ? '' : 'display:none' }, show);
  const scanWrap = h('div', { style: desktop || ceremony.mode === 'scan' ? '' : 'display:none' }, scan);
  const modeSeg = seg([['show', 'Show'], ['scan', 'Scan']], ceremony.mode, (v) => {
    ceremony.mode = v;
    showWrap.style.display = v === 'show' ? '' : 'none';
    scanWrap.style.display = v === 'scan' ? '' : 'none';
    if (v !== 'scan') stopScanner(); else { scanWrap.replaceChildren(phase === 'commit' ? camera(feedText, 'Point at another member\'s code') : fountainReceiver(feedBytes, 'Point at another member\'s animation').el); }
  });
  if (desktop) modeSeg.classList.add('desktop-hide');
  if (!desktop && ceremony.mode !== 'scan') stopScanner();
  screen({
    title: phase === 'commit' ? 'Commit' : 'Reveal', sub: phase === 'commit' ? 'step 1 of 3 · everyone shows, everyone scans' : 'step 2 of 3 · keep both screens still',
    back: () => { if (confirm('Leave the ceremony? Nothing is kept.')) { ceremony = null; go('#/'); } },
    menu: () => sheet(
      h('h2', {}, phase === 'commit' ? 'Share your commitment another way' : 'Share your reveal another way'),
      copyBtn(text), downloadBtn(fileName, text),
      h('h2', {}, 'Receive another way'),
      btn('Paste a code', () => pasteSheet(phase === 'commit' ? 'cahoots-commit:…' : 'cahoots-reveal:…', feedText)),
      (() => { const f = h('input', { type: 'file', style: 'display:none', onchange: async (e) => { if (e.target.files[0]) feedText(await e.target.files[0].text()); } }); return h('div', {}, f, btn('Load a file', () => f.click())); })(),
      btn('Leave the ceremony', () => { ceremony = null; go('#/'); }, 'danger')),
    body: [chips, modeSeg, h('div', { class: 'pair' }, showWrap, scanWrap), st],
  });
}

function screenWords() {
  const { c, label, params } = ceremony;
  const me = c.index();
  const reveal = c.reveal(); // kept so members still collecting can scan it from here
  const out = c.finish(label);
  ceremony = null;
  const words = out.words().split(' ');
  const id = out.circle_id_hex();
  const revealText = enc.reveal(me, reveal);
  const showReveal = () => sheet(h('h2', {}, 'Still collecting? Show them this.'), animatedQr(packReveal(me, reveal), `seat ${me + 1} · reveal`), copyBtn(revealText), downloadBtn(`reveal-${me}.txt`, revealText));
  screen({
    title: 'Check words', sub: 'step 3 of 3',
    menu: showReveal,
    body: [
      h('div', {},
        h('h1', {}, 'Read these out loud.'),
        h('p', { class: 'lead' }, 'One person reads. Everyone else checks their own screen shows the same six, in the same order.')),
      h('div', { class: 'panel' },
        h('div', { class: 'cap' }, h('span', {}, params.name), h('span', {}, `${label} · ${id.slice(0, 8)}`)),
        h('div', { class: 'words' }, ...words.map((w, i) => h('div', {}, h('small', {}, String(i + 1)), w)))),
    ],
    actions: [
      btn('They match', async () => { const circle = out.into_circle(); live.set(id, circle); await saveCircle(circle); go(`#/c/${id}`); }, 'primary'),
      btn('They differ · start over', () => go('#/'), 'danger'),
      link('Someone still needs my reveal', showReveal),
    ],
  });
}

// ---------------------------------------------------------------- a circle: hub, send, read, history, settings
async function screenCircle(id) {
  const circle = await openCircle(id);
  if (!circle) { screen({ title: 'Cahoots', body: [h('p', {}, 'Unknown cahoot.')], actions: [btn('Home', () => go('#/'), 'primary')] }); return; }
  const label = circle.label();
  const low = circle.remaining() < circle.slice_len() / 10;
  screen({
    title: circle.name(), sub: `${label} · you are seat ${circle.my_slice() + 1} of ${circle.members()}`, back: '#/',
    menu: () => sheet(
      h('h2', {}, circle.name()),
      h('p', { class: 'muted' }, CLAIM[label]),
      h('p', { class: 'muted' }, `Inside the cahoot, a message proves a member wrote it, not which member. ${kib(circle.remaining())} of ${kib(circle.slice_len())} of your share is left.`),
      h('p', { class: 'muted' }, 'Plaintext history stays on this device. Pad bytes are erased as they are used, but erasure on phone storage is best effort.'),
      btn('Delete this cahoot from this device', async () => { if (confirm('Delete? The pad and history on this device are removed.')) { live.delete(id); await deleteCircle(id); go('#/'); } }, 'danger')),
    body: [
      h('div', {},
        low ? notice('Less than 10% of your share is left. Plan a new ceremony.') : null,
        h('div', { class: 'rows' },
          h('a', { class: 'row-item', href: `#/chat/${id}` }, h('div', {}, h('b', {}, 'Chat'), h('span', {}, 'end-to-end encrypted, pad-hedged · needs a relay')), h('span', { class: 'arrow' }, '›')),
          h('a', { class: 'row-item', href: `#/c/${id}/send` }, h('div', {}, h('b', {}, 'Send a one-time-pad message'), h('span', {}, 'short · unbreakable · one byte of pad per byte')), h('span', { class: 'arrow' }, '›')),
          h('a', { class: 'row-item', href: `#/c/${id}/read` }, h('div', {}, h('b', {}, 'Read a message'), h('span', {}, 'scan a code or paste')), h('span', { class: 'arrow' }, '›')),
          h('a', { class: 'row-item', href: `#/c/${id}/history` }, h('div', {}, h('b', {}, 'History'), h('span', {}, 'messages on this device')), h('span', { class: 'arrow' }, '›')))),
    ],
  });
}

async function screenSend(id) {
  const circle = await openCircle(id);
  if (!circle) return go('#/');
  let type = 0, padOn = true;
  const text = h('textarea', { class: 'input', placeholder: 'Up to 255 bytes with padding on', style: 'font-family:inherit;font-size:16px' });
  const err = status();
  screen({
    title: 'One-time-pad message', sub: `${kib(circle.remaining())} of your share left`, back: `#/c/${id}`,
    body: [
      h('div', {},
        h('h1', {}, 'Write it.'),
        field('Message', text),
        field('Kind', seg(TYPES.map((t, i) => [i, t]), 0, (v) => { type = Number(v); })),
        h('label', { class: 'toggle' }, h('span', {}, h('b', {}, 'Hide the length'), h('br'), h('span', { class: 'muted' }, 'Pads to 32, 64, 128, or 256 bytes.')), h('input', { type: 'checkbox', checked: true, onchange: (e) => { padOn = e.target.checked; } })),
        err),
    ],
    actions: [btn('Encrypt and spend', async () => {
      try {
        let pt = utf8.encode(text.value);
        if (padOn) { const p = bucket_pad(pt); if (!p) throw 'Too long for padding (255 bytes max). Shorten it or turn padding off.'; pt = p; }
        const wire = circle.send(type, pt);
        await saveCircle(circle); // the range is spent in memory already; the wire leaves only after the ledger is saved
        await saveMessage({ circle: id, dir: 'out', type: TYPES[type], text: text.value, ts: Date.now() });
        screenShowWire(id, circle, wire, 32 + pt.length);
      } catch (e) { err.textContent = ERRORS[e] || String(e); }
    }, 'primary')],
  });
}
function screenShowWire(id, circle, wire, spent) {
  screen({
    title: 'Encrypted', sub: `spent ${spent} bytes · ${kib(circle.remaining())} left`, back: `#/c/${id}`,
    menu: () => sheet(h('h2', {}, 'Share another way'), copyBtn(wire), downloadBtn('message.txt', wire)),
    body: [panelQr(wire, circle.name(), 'anyone in the cahoot can scan this'), h('p', { class: 'muted', style: 'text-align:center' }, 'Post it anywhere. Only your cahoot can read it.')],
    actions: [btn('Done', () => go(`#/c/${id}`), 'primary'), link('Copy as text', async () => { await navigator.clipboard.writeText(wire); })],
  });
}
async function screenRead(id) {
  const circle = await openCircle(id);
  if (!circle) return go('#/');
  const st = status();
  const take = async (wire) => {
    if (wire.startsWith('cahoots-')) return;
    try {
      const r = circle.receive(wire.trim());
      await saveCircle(circle);
      const pt = r.plaintext();
      const un = bucket_unpad(pt);
      const shown = utf8d.decode(un ?? pt);
      await saveMessage({ circle: id, dir: 'in', type: TYPES[r.msg_type()], text: shown, slice: r.slice(), ts: Date.now() });
      screen({
        title: 'Decrypted', sub: `${TYPES[r.msg_type()]} · from seat ${r.slice() + 1}`, back: `#/c/${id}`,
        body: [h('div', { class: 'panel' }, h('div', { class: 'cap' }, h('span', {}, circle.name()), h('span', {}, 'one-time pad')), h('p', { style: 'font-size:20px;margin:0;white-space:pre-wrap' }, shown))],
        actions: [btn('Read another', () => screenRead(id), 'primary'), btn('Done', () => go(`#/c/${id}`))],
      });
    } catch (e) { st.textContent = ERRORS[e] || `Refused: ${e}`; }
  };
  screen({
    title: 'Read a message', back: `#/c/${id}`,
    menu: () => pasteSheet('Paste a message (base64url)', take),
    body: [camera(take, 'Point at the message code'), st],
    actions: [link('Paste instead', () => pasteSheet('Paste a message (base64url)', take))],
  });
}
async function screenHistory(id) {
  const circle = await openCircle(id);
  if (!circle) return go('#/');
  const msgs = (await loadMessages(id)).reverse();
  screen({
    title: 'History', sub: circle.name(), back: `#/c/${id}`,
    body: [msgs.length
      ? h('div', {}, ...msgs.map((m) => h('div', { class: 'msg' }, h('div', { class: 'meta' }, `${m.dir === 'out' ? 'sent' : `from seat ${m.slice + 1}`} · ${m.type} · ${new Date(m.ts).toLocaleString()}`), h('div', { style: 'white-space:pre-wrap' }, m.text))))
      : h('div', { style: 'text-align:center' }, h('h1', {}, 'Nothing yet.'), h('p', { class: 'lead' }, 'Messages you send or read show up here.'))],
  });
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
  const relayBase = () => (settings.relay.trim() || location.origin).replace(/\/$/, '');
  const st = status();
  const list = h('div', { class: 'chatlist' });
  const inp = input({ type: 'text', placeholder: 'Message', maxlength: '4000', autocomplete: 'off' });
  const sendBtn = btn('Send', () => doSend(), 'primary');
  const epoch = () => { const e = circle.current_epoch(); return e ? `epoch ${e}` : 'no epoch yet'; };
  const bubble = (m) => m.dir === 'rejected'
    ? h('div', { class: 'bubble rejected' }, `rejected · ${ERRORS[m.reason] || m.reason} · seq ${m.seq}`)
    : h('div', { class: `bubble ${m.dir}` }, h('div', { class: 'meta' }, `${m.dir === 'out' ? 'you' : `seat ${m.sender + 1}`} · ${new Date(m.ts).toLocaleTimeString()}`), h('div', {}, m.text));
  const append = (m) => { list.append(bubble(m)); list.scrollTop = list.scrollHeight; };
  for (const m of await loadChat(id)) append(m);
  const titleSub = h('small', {}, `${CHAT_LABEL} · ${epoch()}`);

  const post = async (bytes) => {
    const res = await fetch(`${relayBase()}/v1/circles/${id}/messages`, { method: 'POST', headers: { 'Content-Type': 'application/octet-stream' }, body: bytes });
    if (!res.ok) throw `relay ${res.status}: ${await res.text()}`;
    return res.json();
  };
  const doSend = async () => {
    const text = inp.value;
    if (!text) return;
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
    } catch (e) { st.textContent = `Send failed: ${ERRORS[e] || e}`; }
    sendBtn.disabled = false;
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
        st.textContent = `relay unreachable (${e}) · retrying`;
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

  const el = screen({
    title: circle.name(), back: `#/c/${id}`,
    menu: () => sheet((close) => {
      const relayIn = input({ type: 'text', value: settings.relay, placeholder: DEFAULT_RELAY });
      return h('div', { style: 'display:grid;gap:12px' },
        h('h2', {}, 'Chat'),
        h('p', { class: 'muted' }, CHAT_CLAIM),
        h('p', { class: 'muted' }, `One epoch costs 32 pad bytes for the whole cahoot. ${kib(circle.remaining())} of your share left. The seat shown is a chain label, not a proof of who typed.`),
        btn('Start a new epoch', async () => { try { circle.rotate_epoch(); await saveCircle(circle); titleSub.textContent = `${CHAT_LABEL} · ${epoch()}`; close(); } catch (e) { st.textContent = ERRORS[e] || String(e); } }),
        field('Relay (Numbers Station) URL', relayIn),
        btn('Save and reconnect', async () => { settings.relay = relayIn.value.trim(); await saveSettings({ id, relay: settings.relay }); close(); chatRun++; screenChat(id); }, 'primary'));
    }),
    body: [list, st],
    actions: [h('div', { class: 'composer' }, inp, sendBtn)],
  });
  el.querySelector('.bar .title').append(titleSub);
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
    } else if (parts[0] === 'c') {
      if (parts[2] === 'send') await screenSend(parts[1]);
      else if (parts[2] === 'read') await screenRead(parts[1]);
      else if (parts[2] === 'history') await screenHistory(parts[1]);
      else await screenCircle(parts[1]);
    } else if (parts[0] === 'chat') await screenChat(parts[1]);
    else await screenHome();
  } catch (e) { screen({ title: 'Cahoots', body: [notice(`Error: ${e}`, 'bad')], actions: [btn('Home', () => go('#/'), 'primary')] }); }
}

await init();
window.addEventListener('hashchange', route);
if ('serviceWorker' in navigator && location.protocol !== 'file:') navigator.serviceWorker.register('./sw.js').catch(() => {});
route();
