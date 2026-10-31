// Channels: a circle's owner publishes to a channel on a relay; anyone with its link reads, and
// people the owner lets in post. Every key is drawn by the owner's device and the relay keeps
// ciphertext only. All crypto is in cahoots-channel (WASM); this module is screens, storage and
// the relay calls.
//
// Routes: #/ch/new/<circle id>, #/ch/<channel id>, #/ch/<channel id>/members,
// #/ch/<channel id>/requests, and the link #/ch/<relay host>/<channel id>/<secret>.
import {
  ChannelOwner, ChannelMember, verify_channel_object, policy_summary, cbor_split, join_request_summary,
  deposit_meta, b64_encode, b64_decode, chat_bucket_pad, chat_bucket_unpad,
} from './pkg/cahoots_wasm.js';

let U; // the shared helpers from app.js
export function setupChannels(ui) { U = ui; }

const LABEL = 'End-to-end encrypted; the relay keeps ciphertext only';
const LINK_LIMIT = 'Anyone you forward this link to can read until you retire it.';
const DEAD = 'This link no longer works. Ask for a new one.';
const WAITING = 'Waiting for the owner to let you post.';
const CLOSED = 'Only people the owner lets in can read this channel.';
const NEW_LINK = 'Everyone who should keep reading needs this new link.';
const OTHER_RELAY = 'This link names a different relay than the one this channel uses.';
const NEEDS_APPROVALS = 'This circle has more than one member; changing the policy needs their approval, which this version cannot collect yet.';
const POLL_WAIT_MS = 25000;
const PRESETS = [
  ['0', 'Announcements', 'Anyone with the link reads. Only you post.'],
  ['1', 'Announcements with approvals', 'Anyone with the link reads. A post goes out when enough seats of your circle approve it.'],
  ['2', 'Group chat', 'Anyone with the link reads and can ask to post.'],
  ['3', 'Members post', 'Anyone with the link reads. People you let in post.'],
  ['4', 'Closed', 'Only people you let in read and post.'],
];
const CH_ERRORS = {
  'needs-approvals': NEEDS_APPROVALS,
  'no-seat-key': 'This circle was made before seat keys. Run a new ceremony to start a channel.',
  relay: 'That relay address cannot go in a link.',
  name: 'Give the channel a name of up to 64 bytes.',
  policy: 'This channel lets only the owner post.',
  evidence: 'This request came with an old link, so it cannot be allowed.',
  actions: 'A request is let in to read, or to read and post.',
  signature: 'This does not carry a valid signature.',
  'unknown-epoch': 'This message uses keys this device does not hold.',
  'no-proposal': 'Write the post and propose it first.',
  digest: 'That is not an approval code.',
  link: 'That link does not read.',
};
const friendly = (e) => {
  if (e instanceof RelayError) return e.message;
  const s = String(e);
  if (CH_ERRORS[s]) return CH_ERRORS[s];
  if (s.startsWith('refused:revoked')) return 'Sent by a member who was removed.';
  if (s.startsWith('refused:') || s.startsWith('grant:')) return 'Sent by someone without a grant to post.';
  return U.friendly(s);
};

// ---------------------------------------------------------------- small helpers
const hexOf = (bytes) => Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('');
const short = (kid) => kid.slice(0, 8);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const epochNumber = (epochHex) => (epochHex ? new DataView(Uint8Array.from(epochHex.slice(32).match(/../g), (x) => parseInt(x, 16)).buffer).getUint32(0, true) : 0);
const isIos = () => /iPhone|iPad|iPod/.test(navigator.userAgent);
const getApp = () => (isIos() ? U.h('p', { class: 'muted ch-get-app' }, U.h('a', { href: U.SITE, target: '_blank', rel: 'noopener' }, 'Get the app')) : null);
/** A canonical CBOR array of byte strings, as the relay and the wasm read them. */
export function cborArray(items) {
  const head = (major, n) => (n < 24 ? [major << 5 | n] : n < 256 ? [major << 5 | 24, n] : n < 65536 ? [major << 5 | 25, n >> 8, n & 255] : [major << 5 | 26, n >>> 24, (n >> 16) & 255, (n >> 8) & 255, n & 255]);
  const parts = [Uint8Array.from(head(4, items.length))];
  for (const it of items) { parts.push(Uint8Array.from(head(2, it.length)), it); }
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let o = 0; for (const p of parts) { out.set(p, o); o += p.length; }
  return out;
}
/** The relay's address from a link's host: plain http for this machine, https for the rest. */
const relayBase = (host) => `${/^(localhost|127\.\d+\.\d+\.\d+|\[::1\])(:\d+)?$/.test(host) ? 'http' : 'https'}://${host}`;
const hostOf = (url) => { try { return new URL(url).host; } catch { return ''; } };
/** A relay address or a link's host as compared: the host lowercased, with its port. */
const relayKey = (relay) => hostOf(/:\/\//.test(relay) ? relay : relayBase(relay)).toLowerCase();

// ---------------------------------------------------------------- the relay
export class RelayError extends Error {
  constructor(status, code, reason) {
    const text = status === 401 ? 'The relay refused the read.'
      : status === 403 ? `The relay refused it (${reason || code}).`
      : status === 404 ? 'The relay does not hold this channel.'
      : status === 429 ? 'The relay is busy. Try again in a moment.'
      : `The relay answered ${status}${code ? ` (${code})` : ''}.`;
    super(text);
    this.status = status; this.code = code; this.reason = reason;
  }
}
/** One relay call. `auth` is `{ gate: hex }` for the link's token, or `{ member }` for a proof by
 * that member's key over `METHOD PATH?QUERY` and a fresh relay nonce. */
async function call(host, method, path, { body, auth } = {}) {
  const headers = {};
  if (body) headers['Content-Type'] = 'application/octet-stream';
  if (auth?.gate) headers['Cahoots-Gate'] = auth.gate;
  else if (auth?.member) {
    const nres = await fetch(`${relayBase(host)}/v1/nonce`);
    if (!nres.ok) throw new RelayError(nres.status);
    const { nonce } = await nres.json();
    const q = path.indexOf('?');
    const target = `${method} ${q < 0 ? path : path.slice(0, q)}?${q < 0 ? '' : path.slice(q + 1)}`;
    headers['Cahoots-Proof'] = auth.member.proof_for(target, nonce);
    headers['Cahoots-Kid'] = auth.member.my_kid_hex();
  }
  const res = await fetch(`${relayBase(host)}${path}`, { method, headers, body });
  if (!res.ok) { let j = {}; try { j = await res.json(); } catch { /* not json */ } throw new RelayError(res.status, j.error, j.reason); }
  if (res.status === 204) return null;
  return (res.headers.get('content-type') || '').includes('json') ? res.json() : new Uint8Array(await res.arrayBuffer());
}
export const chGet = (host, path, auth) => call(host, 'GET', path, { auth });
export const chPost = (host, path, bytes) => call(host, 'POST', path, { body: bytes });
const chPut = (host, path, bytes) => call(host, 'PUT', path, { body: bytes });
const chDelete = (host, path, bytes) => call(host, 'DELETE', path, { body: bytes });
/** A message post; a 409 replay means the relay already holds it. */
async function postMessage(host, id, wire) {
  try { return await chPost(host, `/v1/channels/${id}/messages`, wire); } catch (e) { if (e.status === 409 && e.code === 'replay') return null; throw e; }
}

// ---------------------------------------------------------------- storage
const putChannel = (rec) => U.tx('channels', 'readwrite', (s) => s.put(rec));
const getChannel = (id) => U.tx('channels', 'readonly', (s) => s.get(id));
export const loadChannels = () => U.tx('channels', 'readonly', (s) => s.getAll());
const deleteChannel = async (id) => {
  await U.tx('channels', 'readwrite', (s) => s.delete(id));
  const msgs = await loadMsgs(id);
  await U.tx('chmsgs', 'readwrite', (s) => { for (const m of msgs) s.delete(m.key); });
};
const saveMsg = (m) => U.tx('chmsgs', 'readwrite', (s) => s.add(m));
const loadMsgs = (id) => U.tx('chmsgs', 'readonly', (s) => s.index('channel').getAll(id));
const loadChSettings = async (id) => (await U.tx('settings', 'readonly', (s) => s.get(`ch:${id}`))) || { id: `ch:${id}` };
const saveChSettings = (row) => U.tx('settings', 'readwrite', (s) => s.put(row));

const live = new Map(); // channel id -> { rec, owner?, member, chains: Map<kid, Uint8Array[]> }
async function openChannel(id) {
  if (live.has(id)) return live.get(id);
  const rec = await getChannel(id);
  if (!rec) return null;
  let e;
  if (rec.role === 'owner') { const owner = ChannelOwner.import(rec.state); e = { rec, owner, member: owner.member(), chains: new Map() }; }
  else e = { rec, member: ChannelMember.import(rec.state), chains: new Map() };
  live.set(id, e);
  return e;
}
/** Write the channel's state (keys, chains, counters) before anything it made leaves the device. */
async function persist(e) {
  e.rec.state = e.owner ? e.owner.export() : e.member.export();
  await putChannel(e.rec);
}
const readAuth = (e) => {
  if (e.owner) return { member: e.member };
  const gate = e.member.gate_token_hex();
  if (gate && !e.useProof) return { gate };
  return e.rec.grantChain ? { member: e.member } : null;
};

// ---------------------------------------------------------------- owner actions on the relay
async function postDepositsAndRotation(e, rotation) {
  const { relay: host, id } = e.rec;
  for (const d of cbor_split(e.owner.deposits())) await chPost(host, `/v1/channels/${id}/deposits`, b64_decode(d));
  await postMessage(host, id, rotation);
}
/** A new epoch: deposits for the live link and every admitted member, then the rotation control. */
async function startNewKeys(e) {
  const rotation = e.owner.rotate();
  await persist(e);
  await postDepositsAndRotation(e, rotation);
}
/** Register the current link's gate token with the relay, if it is not registered yet. */
async function registerGate(e) {
  const hash = e.owner.gate_token_hash_hex();
  if (!hash || e.rec.gateHash === hash) return false;
  await chPost(e.rec.relay, `/v1/channels/${e.rec.id}/gates`, e.owner.gate_registration());
  e.rec.gateHash = hash;
  await persist(e);
  return true;
}
/** Retire the live link: the relay refuses its token, and later epochs have no deposit for it.
 * The secret is forgotten at once; the signed statement stays in the owner's state until the relay
 * confirms, and a DELETE that fails here is sent again on each open of the channel. */
async function retireLink(e) {
  if (!e.owner.gate_token_hash_hex()) return false;
  e.owner.retire_link();
  e.rec.gateHash = null;
  await persist(e);
  await flushRetires(e);
  return true;
}
/** Send each retire the relay has not confirmed; a 204, or a 404 (the token is gone), clears it. */
async function flushRetires(e) {
  for (const p of JSON.parse(e.owner.pending_retire_json())) {
    try { await chDelete(e.rec.relay, `/v1/channels/${e.rec.id}/gates/${p.hash}`, b64_decode(p.statement)); }
    catch (err) { if (err.status !== 404) throw err; }
    e.owner.retire_confirmed(p.hash);
    await persist(e);
  }
}
/** Post the policy `set_policy` made, the reissued channel object, and the policy change control. */
async function publishPolicy(e, body) {
  const { relay: host, id } = e.rec;
  const control = e.owner.policy_change_control();
  e.rec.policy = e.owner.policy();
  e.rec.policyLine = policy_summary(e.rec.policy);
  await persist(e);
  await chPost(host, `/v1/channels/${id}/policy`, body);
  await chPost(host, '/v1/channels', e.owner.channel_object());
  await postMessage(host, id, control);
}
/** Admit one join request with `actions`, 'read' or 'read,post' (never admit or govern); a `post`
 * the policy does not allow falls back to read. */
async function admitRequest(e, bytes, actions) {
  if (actions !== 'read' && actions !== 'read,post') throw 'actions';
  let parts;
  try { parts = cbor_split(e.owner.admit(bytes, actions)); }
  catch (err) {
    if (String(err) !== 'policy' || !actions.includes('post')) throw err;
    actions = 'read'; parts = cbor_split(e.owner.admit(bytes, actions));
  }
  const kid = JSON.parse(join_request_summary(bytes)).kid;
  const m = JSON.parse(e.owner.members_json()).find((x) => x.kid === kid);
  e.rec.actions = { ...(e.rec.actions || {}), [m.grant_id]: actions };
  await persist(e);
  const { relay: host, id } = e.rec;
  await chPost(host, `/v1/channels/${id}/grants`, b64_decode(parts[0]));
  await chPost(host, `/v1/channels/${id}/deposits`, b64_decode(parts[1]));
  await postMessage(host, id, b64_decode(parts[2]));
  return actions;
}
async function fetchRequests(e) {
  const j = await chGet(e.rec.relay, `/v1/channels/${e.rec.id}/requests`, { member: e.member });
  const handled = new Set(e.rec.handled || []);
  return (j.requests || []).filter((r) => !handled.has(r.id));
}
const markHandled = async (e, rid) => { e.rec.handled = [...(e.rec.handled || []), rid]; await putChannel(e.rec); };
/** With the toggle on, let in a request only when the live link grants every action it asks for
 * (`permitted` covers it). Any other request stays in the list for the owner to decide. */
async function autoAllow(e) {
  const settings = await loadChSettings(e.rec.id);
  if (!settings.autoAllow) return 0;
  let n = 0;
  for (const r of await fetchRequests(e)) {
    const bytes = b64_decode(r.body);
    try {
      const actions = e.owner.permitted(bytes);
      const granted = actions ? actions.split(',') : [];
      const asked = JSON.parse(join_request_summary(bytes)).actions.split(',').map((a) => a.trim());
      if (!granted.length || !asked.every((a) => granted.includes(a))) continue;
      await admitRequest(e, bytes, actions); await markHandled(e, r.id); n++;
    } catch (err) { if (['malformed', 'signature', 'evidence', 'wrong-channel'].includes(String(err))) await markHandled(e, r.id); }
  }
  return n;
}

// ---------------------------------------------------------------- reading
async function chainsFor(e, kid, fresh) {
  if (!fresh && e.chains.has(kid)) return e.chains.get(kid);
  const j = await chGet(e.rec.relay, `/v1/channels/${e.rec.id}/grants?kid=${kid}`, readAuth(e));
  const list = (j.grants || []).map((g) => b64_decode(g));
  e.chains.set(kid, list);
  return list;
}
/** Install one signed deposit once its signer's grant chain (fetched by the deposit's kid) verifies
 * and carries govern. With `ids`, the deposits a rotation names, any other deposit is passed over. */
async function installDeposit(e, b64, byLink, ids) {
  const bytes = b64_decode(b64);
  let meta;
  try { meta = JSON.parse(deposit_meta(bytes)); } catch { return false; }
  if (ids && !ids.includes(meta.id)) return false;
  for (const fresh of [false, true]) {
    for (const chain of await chainsFor(e, meta.kid, fresh)) {
      try {
        if (byLink) e.member.install_from_link_deposit(bytes, chain, e.rec.manifest);
        else e.member.install_from_key_deposit(bytes, chain, e.rec.manifest);
        return true;
      } catch { /* not ours, or not from a key that governs: try the next */ }
    }
  }
  return false;
}
/** Fetch and open this member's deposit for an epoch: the link's first, then one sealed to its
 * key. A refused link token (the link was just retired) falls back to a proof by a member with a
 * grant; the key deposit is always tried before giving up. A deposit whose signer does not check
 * out is passed over for the next. */
async function fetchEpoch(e, epochHex, ids = null) {
  const { relay: host, id } = e.rec;
  const deposits = async (to) => {
    const path = `/v1/channels/${id}/deposits?to=${to}&epoch=${epochHex}`;
    const auth = readAuth(e);
    if (!auth) return [];
    try { return (await chGet(host, path, auth)).deposits || []; }
    catch (err) {
      if (err.status !== 401) throw err;
      if (!auth.gate || !e.rec.grantChain) return [];
      e.useProof = true;
      try { return (await chGet(host, path, { member: e.member })).deposits || []; }
      catch (err2) { if (err2.status !== 401) throw err2; return []; }
    }
  };
  const linkId = e.member.link_id_hex();
  if (linkId) {
    for (const d of await deposits(linkId)) if (await installDeposit(e, d, true, ids)) return true;
  }
  for (const d of await deposits(e.member.my_kid_hex())) if (await installDeposit(e, d, false, ids)) return true;
  return false;
}
const isGrantError = (s) => /^(grant:|refused:|signature|no-admit|wrong-channel|no-grant)/.test(s);
/** Read one entry of the stream; returns a message to store, or null. */
async function take(e, entry) {
  const bytes = b64_decode(entry.message);
  if (bytes[0] !== 4 || bytes.length < 78) return null;
  const kid = hexOf(bytes.subarray(37, 69));
  const epochHex = hexOf(bytes.subarray(17, 37));
  e.maxEpoch = Math.max(e.maxEpoch || 0, epochNumber(epochHex));
  if (kid === e.member.my_kid_hex()) return null; // our own: stored when it was sent
  let r = null, error = 'no-grant';
  for (const fresh of [false, true]) {
    for (const chain of await chainsFor(e, kid, fresh)) {
      try { r = e.member.receive(entry.message, chain, e.rec.manifest); break; }
      catch (err) {
        error = String(err);
        if (error === 'unknown-epoch' && !e.owner && await fetchEpoch(e, epochHex)) {
          try { r = e.member.receive(entry.message, chain, e.rec.manifest); break; } catch (err2) { error = String(err2); }
        }
        if (!isGrantError(error)) break;
      }
    }
    if (r || !isGrantError(error)) break;
  }
  const base = { channel: e.rec.id, kid, ts: entry.received_at, seq: entry.seq };
  if (!r) return error === 'unknown-epoch' ? null : { ...base, dir: 'rejected', text: error, type: 'text', ok: false };
  const type = r.msg_type();
  if (type === 1) {
    e.rec.ownerKids = [...new Set([...(e.rec.ownerKids || []), kid])];
    const c = JSON.parse(r.control_json());
    if (c.kind === 'rotation' && !e.owner) {
      // The control is already read; a failed fetch is retried after the sync, never by reading it again.
      try { if (!(await fetchEpoch(e, c.epoch, c.deposits))) e.behind = true; } catch { e.retryEpoch = c.epoch; }
    }
    if (c.kind === 'policy') await refreshPolicy(e);
    if (c.kind === 'revocation') return { ...base, dir: 'note', text: 'The owner removed a member and will start new keys.', type: 'control', ok: true };
    if (c.kind === 'policy') return { ...base, dir: 'note', text: `The policy changed: ${e.rec.policyLine}.`, type: 'control', ok: true };
    return null;
  }
  const raw = r.text();
  const text = U.utf8d.decode(chat_bucket_unpad(raw) ?? raw);
  return { ...base, dir: 'in', name: nameFor(e, kid), text, type: type === 2 ? 'announcement' : 'text', ok: type === 2 ? r.announcement_ok() : true };
}
function nameFor(e, kid) {
  if (e.owner) { const m = JSON.parse(e.owner.members_json()).find((x) => x.kid === kid); return m ? m.name : short(kid); }
  return (e.rec.ownerKids || []).includes(kid) ? 'owner' : short(kid);
}
/** Take the policy the relay now holds, once it checks out: the channel object verifies against the
 * owner circle's manifest this browser keeps, names this channel, and names the policy by its hash. */
async function refreshPolicy(e) {
  try {
    const j = await (await fetch(`${relayBase(e.rec.relay)}/v1/channels/${e.rec.id}`)).json();
    if (!j.policy || !j.channel) return;
    const policy = b64_decode(j.policy);
    const obj = JSON.parse(verify_channel_object(b64_decode(j.channel), e.rec.manifest));
    if (obj.channel_id !== e.rec.id) return;
    if (hexOf(new Uint8Array(await crypto.subtle.digest('SHA-256', policy))) !== obj.policy_id) return;
    e.member.set_policy(policy);
    e.rec.policy = policy;
    e.rec.policyLine = policy_summary(policy);
  } catch { /* keep the policy we hold */ }
}
async function markDead(e) { e.rec.dead = true; await persist(e); }

// ---------------------------------------------------------------- screens
let chRun = 0;
window.addEventListener('hashchange', () => { chRun++; });

export async function routeChannels(parts) {
  // parts[0] is 'ch'
  if (parts[1] === 'new') return screenStart(parts[2]);
  if (parts.length >= 3 && /[.:]/.test(parts[1])) return screenJoin(parts[1], parts[2], parts[3]);
  if (parts[2] === 'members') return screenMembers(parts[1]);
  if (parts[2] === 'requests') return screenRequests(parts[1]);
  return screenChannel(parts[1]);
}

async function screenStart(circleId) {
  const { h, field, input, seg, btn, status, screen } = U;
  const generation = U.generation();
  const circle = await U.openCircle(circleId);
  if (!circle) return U.go('#/');
  const settings = await U.loadSettings(circleId);
  if (generation !== U.generation()) return;
  const host = hostOf(settings.relay);
  let preset = '0';
  const name = input({ type: 'text', maxlength: '64', autocomplete: 'off', placeholder: 'e.g. Garden news' });
  const hint = h('span', { class: 'hint' }, PRESETS[0][2]);
  const sentence = h('p', { class: 'muted' }, 'The relay keeps encrypted messages only. Anyone with the link can read.');
  const err = status();
  const start = btn('Start', async () => {
    const n = name.value.trim();
    if (!n) { err.textContent = 'Give the channel a name.'; name.focus(); return; }
    start.disabled = true;
    try {
      err.textContent = 'Checking the relay…';
      const info = await (await fetch(`${relayBase(host)}/v1/info`)).json();
      if (!info.features?.includes('channels') || !info.features?.includes('gate')) throw `${host} does not carry channels yet. Point this circle at a relay that does, from its relay address.`;
      const owner = circle.channel_owner(n, Number(preset), 0);
      const id = owner.channel_id_hex();
      const closed = preset === '4';
      if (!closed) owner.link_url(host);
      const rec = { id, role: 'owner', circle: circleId, relay: host, name: n, preset: Number(preset), quorum: preset === '1' ? majority(circle.members()) : 0, policy: owner.policy(), policyLine: policy_summary(owner.policy()), grantChain: owner.self_grant(), manifest: owner.manifest(), seq: null, kid: owner.member().my_kid_hex(), handled: [], actions: {}, gateHash: null };
      const e = { rec, owner, member: owner.member(), chains: new Map() };
      live.set(id, e);
      await persist(e);
      err.textContent = 'Uploading the channel…';
      await chPut(host, `/v1/circles/${circleId}/manifest`, owner.manifest());
      await chPost(host, '/v1/channels', owner.channel_object());
      await chPost(host, `/v1/channels/${id}/policy`, owner.policy_body());
      await chPost(host, `/v1/channels/${id}/grants`, owner.self_grant());
      if (!closed) await registerGate(e);
      await startNewKeys(e);
      screenShare(e, { first: true });
    } catch (x) { err.textContent = friendly(x); start.disabled = false; }
  }, 'primary');
  screen({
    title: 'Start a channel', sub: circle.name(), back: `#/c/${circleId}`,
    body: [h('div', { class: 'stack' },
      h('h1', {}, 'Start a channel.'),
      h('p', { class: 'lead' }, 'Post to everyone who has the link. Your circle owns it; this device holds its keys.'),
      field('Channel name', name),
      h('div', { class: 'field' }, h('span', { class: 'field-label' }, 'Who reads and posts'),
        seg(PRESETS.map(([v, l]) => [v, l]), preset, (v) => {
          preset = v; hint.textContent = PRESETS[Number(v)][2];
          sentence.textContent = v === '4' ? 'The relay keeps encrypted messages only. Only people you let in can read.' : 'The relay keeps encrypted messages only. Anyone with the link can read.';
        }), hint),
      sentence,
      h('p', { class: 'muted' }, `Relay: ${host}`),
      err)],
    actions: [start],
  }).classList.add('ch-start');
  setTimeout(() => name.focus(), 50);
}

const linkFor = (e) => (e.owner.gate_token_hash_hex() ? e.owner.link_url(e.rec.relay) : null);
function shareButtons(url) {
  const { btn } = U;
  return [
    navigator.share ? btn('Share the link', async () => { try { await navigator.share({ title: 'Cahoots channel', url }); } catch { /* closed */ } }, 'primary') : null,
    btn('Copy the link', async () => { await navigator.clipboard.writeText(url); U.toast('Copied'); }, navigator.share ? '' : 'primary'),
  ];
}
function screenShare(e, { first = false, note = null } = {}) {
  const { h, btn, screen } = U;
  const url = linkFor(e);
  // The share screen can sit on the channel's own address, where a hash change would not fire.
  const open = () => { if (location.hash === `#/ch/${e.rec.id}`) { chRun++; screenChannel(e.rec.id); } else U.go(`#/ch/${e.rec.id}`); };
  screen({
    title: first ? 'Channel started' : note ? 'A new link' : 'Share the link', sub: e.rec.name, back: open,
    body: [h('div', { class: 'stack' },
      h('h1', {}, first ? `${e.rec.name} is ready.` : note ? 'A new link.' : 'Share the link.'),
      note ? U.notice(note) : null,
      url ? h('p', { class: 'lead' }, LINK_LIMIT) : h('p', { class: 'lead' }, 'This channel has no link. Only people you let in can read.'),
      url ? h('div', { class: 'ch-link' }, url) : null,
      h('p', { class: 'muted' }, `${e.rec.policyLine}.`))],
    actions: [...(url ? shareButtons(url) : []), btn('Open the channel', open, url ? '' : 'primary')],
  });
}

async function screenJoin(host, id, secret) {
  const { h, field, input, btn, status, screen } = U;
  const generation = U.generation();
  const fragment = `#/ch/${host}/${id}${secret ? `/${secret}` : ''}`;
  const existing = await getChannel(id);
  if (existing && (existing.role === 'owner' || !secret)) { location.replace(`#/ch/${id}`); return; }
  if (existing) {
    // The channel's keys, cursor and grant belong to the relay it was read on.
    if (relayKey(host) !== relayKey(existing.relay)) { U.toast(OTHER_RELAY, 'bad'); location.replace(`#/ch/${id}`); return; }
    // A newer link for a channel this browser already reads: take its secret, keep the key and grant.
    const e = await openChannel(id);
    const probe = ChannelMember.from_link(fragment);
    const newer = probe.gate_token_hex() !== e.member.gate_token_hex();
    probe.free();
    if (newer || e.rec.dead) {
      try { e.member.set_link(fragment); } catch (x) { U.toast(friendly(x), 'bad'); location.replace(`#/ch/${id}`); return; }
      Object.assign(e.rec, { relay: host, dead: false });
      Object.assign(e, { useProof: false, behind: false, maxEpoch: 0, retryEpoch: null });
      await persist(e);
    }
    location.replace(`#/ch/${id}`);
    return;
  }
  const err = status();
  screen({ title: 'Channel', body: [h('div', { class: 'empty' }, h('p', { class: 'muted' }, `Reading ${host}…`), err)] });
  let obj, policy, manifest, line;
  try {
    const res = await fetch(`${relayBase(host)}/v1/channels/${id}`);
    if (!res.ok) throw new RelayError(res.status);
    const j = await res.json();
    if (!j.policy) throw 'The relay holds no policy for this channel.';
    [policy, manifest] = [b64_decode(j.policy), b64_decode(j.manifest)];
    obj = JSON.parse(verify_channel_object(b64_decode(j.channel), manifest));
    if (obj.channel_id !== id) throw 'The relay answered with another channel.';
    const policyId = hexOf(new Uint8Array(await crypto.subtle.digest('SHA-256', policy)));
    if (policyId !== obj.policy_id) throw 'The relay\'s copy of this channel does not check out.';
    line = policy_summary(policy);
  } catch (x) {
    if (generation !== U.generation()) return;
    screen({ title: 'Channel', back: '#/', body: [h('div', { class: 'empty' }, h('h1', {}, 'This channel cannot be opened.'), h('p', { class: 'lead' }, friendly(x)), getApp())], actions: [btn('Home', () => U.go('#/'), 'primary')] });
    return;
  }
  if (generation !== U.generation()) return;
  if (!secret) {
    screen({ title: obj.name, sub: 'a channel', back: '#/', body: [h('div', { class: 'empty' }, h('h1', {}, obj.name), h('p', { class: 'lead' }, CLOSED), h('p', { class: 'muted' }, `${line[0].toUpperCase()}${line.slice(1)}.`), getApp())], actions: [btn('Home', () => U.go('#/'), 'primary')] });
    return;
  }
  const member = ChannelMember.from_link(fragment);
  member.set_policy(policy);
  const linkPosts = member.may_ask_to_post();
  const you = input({ type: 'text', maxlength: '64', autocomplete: 'off', placeholder: 'How the owner will know you' });
  screen({
    title: obj.name, sub: 'a channel', back: '#/',
    body: [h('div', { class: 'stack' },
      h('p', { class: 'eyebrow' }, 'Channel'),
      h('h1', {}, obj.name),
      h('p', { class: 'muted' }, `Owned by circle ${obj.circle_id.slice(0, 8)} · on ${host}`),
      h('p', { class: 'lead' }, `${line[0].toUpperCase()}${line.slice(1)}.`),
      h('p', { class: 'muted' }, `${LABEL}.`),
      linkPosts ? field('Your name', you, 'Shown to the owner when you ask to post, and to members beside your posts.') : null,
      err, getApp())],
    actions: [btn('Open', async () => {
      try {
        const myName = you.value.trim();
        if (linkPosts && !myName) { err.textContent = 'Give a name, so the owner knows who is asking.'; you.focus(); return; }
        const rec = { id, role: 'member', circle: obj.circle_id, relay: host, name: obj.name, policy, policyLine: line, grantChain: null, manifest, seq: null, kid: member.my_kid_hex(), myName, wantsPost: linkPosts, requested: false, dead: false };
        const e = { rec, member, chains: new Map() };
        live.set(id, e);
        await persist(e);
        location.replace(`#/ch/${id}`); // the link's secret leaves the address bar and the history
      } catch (x) { err.textContent = friendly(x); }
    }, 'primary')],
  });
}

async function screenChannel(id) {
  const { h, btn, input, status, screen, sheet } = U;
  const generation = U.generation();
  const e = await openChannel(id);
  if (!e) return U.go('#/');
  const history = await loadMsgs(id);
  if (generation !== U.generation()) return;
  const run = ++chRun;
  const rec = e.rec;
  const st = status();
  const list = h('div', { class: 'chatlist' });
  const empty = h('div', { class: 'chat-empty' }, h('h1', {}, 'Nothing posted yet.'), h('p', {}, e.owner ? 'Write the first post. Everyone with the link reads it.' : 'Posts show up here as they arrive.'));
  const bubble = (m) => {
    if (m.dir === 'rejected') return h('div', { class: 'bubble rejected' }, `refused · ${friendly(m.text)} · seq ${m.seq}`);
    if (m.dir === 'note') return h('div', { class: 'bubble note' }, m.text);
    const who = m.dir === 'out' ? 'you' : m.name || short(m.kid);
    const mark = m.type === 'announcement' ? (m.ok ? ' · announcement' : ' · announcement, not approved') : '';
    return h('div', { class: `bubble ${m.dir}${m.type === 'announcement' ? ' announcement' : ''}` }, h('div', { class: 'meta' }, `${who} · ${new Date(m.ts).toLocaleTimeString()}${mark}`), h('div', {}, m.text));
  };
  const append = (m) => { empty.remove(); list.append(bubble(m)); list.scrollTop = list.scrollHeight; };
  if (!history.length) list.append(empty);
  for (const m of history) append(m);

  const preset = rec.preset ?? -1;
  const proposing = e.owner && preset === 1;
  const inp = input({ type: 'text', placeholder: proposing ? 'An announcement' : 'Message', maxlength: '4000', autocomplete: 'off', enterkeyhint: 'send', 'aria-label': 'Message' });
  const sendBtn = btn(proposing ? 'Propose' : 'Send', () => doSend(), 'primary');
  const banner = h('div', { class: 'ch-banner' });
  // Posting follows the grant: one to read only leaves the composer off.
  const canPost = () => !rec.dead && (e.owner || (!!rec.grantChain && e.member.may_post()));
  const refresh = () => {
    inp.disabled = sendBtn.disabled = !canPost();
    banner.replaceChildren();
    if (rec.dead) banner.append(U.notice(DEAD, 'bad'));
    else if (!e.owner && !rec.grantChain && rec.requested) banner.append(U.notice(WAITING));
    else if (!e.owner && rec.grantChain && !canPost()) banner.append(U.notice('The owner let you read this channel, not post.'));
    else if (!e.owner && !rec.grantChain && !rec.wantsPost) banner.append(U.notice('You can read this channel. Only the owner posts.'));
    else if (proposing) banner.append(U.notice(approvalsNeeded(e) > 1 ? `This channel needs ${approvalsNeeded(e)} approvals to post.` : 'Each post carries your approval.'));
  };

  const doSend = async () => {
    const text = inp.value;
    if (!text.trim() || !canPost()) return;
    if (proposing) { proposeSheet(e, text, () => { inp.value = ''; }, append); return; }
    sendBtn.disabled = inp.disabled = true;
    try {
      const pt = chat_bucket_pad(U.utf8.encode(text));
      if (!pt) throw 'Message too long.';
      const wire = e.member.send(pt);
      await persist(e); // the chain step is recorded before the ciphertext leaves
      const res = await postMessage(rec.relay, id, b64_decode(wire));
      const m = { channel: id, dir: 'out', kid: rec.kid, name: 'you', text, type: 'text', ts: Date.now(), seq: res?.seq ?? null, ok: true };
      await saveMsg(m); append(m); inp.value = '';
    } catch (x) { st.textContent = `Not sent: ${friendly(x)}`; }
    refresh();
    if (run === chRun) inp.focus();
  };
  inp.onkeydown = (ev) => { if (ev.key === 'Enter') doSend(); };

  const ownerMenu = () => sheet((close) => h('div', { class: 'stack' },
    h('h2', {}, rec.name), h('p', { class: 'muted' }, `${LABEL}; ${rec.policyLine}.`),
    U.btn('Share the link', async () => { close(); await shareFlow(e); }),
    e.owner.gate_token_hash_hex() ? U.btn('Retire the link', async () => {
      if (!confirm('Retire this link? Everyone reading by it stops at the next keys. People you let in keep reading.')) return;
      close();
      try { await retireLink(e); await startNewKeys(e); U.toast('The link is retired and new keys started.', 'ok'); } catch (x) { st.textContent = friendly(x); }
    }) : null,
    U.btn('Start new keys', async () => { close(); try { await startNewKeys(e); U.toast('New keys started.', 'ok'); } catch (x) { st.textContent = friendly(x); } }),
    U.btn('Policy', () => { close(); policySheet(e); }),
    U.btn('Members', () => { close(); U.go(`#/ch/${id}/members`); }),
    U.btn('Requests', () => { close(); U.go(`#/ch/${id}/requests`); }),
    U.btn('Approve a post', () => { close(); approveSheet(e); }),
    removeButton(e, close)));
  const memberMenu = async () => {
    const circle = await circleFor(e);
    sheet((close) => h('div', { class: 'stack' },
      h('h2', {}, rec.name), h('p', { class: 'muted' }, `${LABEL}; ${rec.policyLine}.`),
      h('p', { class: 'muted' }, `On ${rec.relay}. Your key: ${short(rec.kid)}.`),
      circle ? U.btn('Approve a post', () => { close(); approveSheet(e, circle); }) : null,
      removeButton(e, close), getApp()));
  };

  const el = screen({
    title: rec.name, sub: e.owner ? 'your channel' : `on ${rec.relay}`, back: '#/', menu: e.owner ? ownerMenu : memberMenu,
    body: [h('p', { class: 'ch-label' }, `${LABEL}; ${rec.policyLine}`), banner, list, st],
    actions: [h('div', { class: 'composer' }, inp, sendBtn)],
  });
  el.classList.add('chat-screen', 'channel-screen');
  refresh();
  if (e.owner) flushRetires(e).catch(() => { /* sent again on the next round */ });
  if (!rec.dead) poll(e, run, { append, refresh, st });
}
const majority = (n) => (n <= 1 ? 1 : Math.floor(n / 2) + 1);
const approvalsNeeded = (e) => Math.max(1, e.rec.quorum || 1);
function removeButton(e, close) {
  return U.btn('Remove from this browser', async () => {
    if (!confirm('Remove this channel from this browser? Its keys go with it.')) return;
    close(); live.delete(e.rec.id); await deleteChannel(e.rec.id); U.go('#/');
  }, 'danger');
}
async function circleFor(e) {
  try { return await U.openCircle(e.rec.circle); } catch { return null; }
}

async function poll(e, run, ui) {
  try { await pollLoop(e, run, ui); }
  catch (err) { if (run === chRun) ui.st.textContent = friendly(err); }
}
async function pollLoop(e, run, ui) {
  const { rec } = e;
  let synced = false;
  while (run === chRun) {
    const auth = readAuth(e);
    if (!auth) { ui.st.textContent = CLOSED; return; } // no link and no grant: nothing to read
    const q = `${rec.seq == null ? '' : `after=${rec.seq}&`}wait=${synced ? POLL_WAIT_MS : 0}`;
    let data;
    try { data = await chGet(rec.relay, `/v1/channels/${rec.id}/messages?${q}`, auth); ui.st.textContent = 'Connected to relay'; }
    catch (err) {
      if (run !== chRun) return;
      if (err.status === 401 && !e.owner) {
        if (auth.gate && rec.grantChain && !e.useProof) { e.useProof = true; continue; }
        await markDead(e); ui.refresh(); ui.st.textContent = ''; return;
      }
      ui.st.textContent = `Cannot reach ${rec.relay}. Retrying.`;
      await sleep(3000); continue;
    }
    if (run !== chRun) return;
    const entries = data.entries || [];
    let failed = false;
    for (const entry of entries) {
      if (run !== chRun) return; // a newer screen owns this channel now
      let m;
      try { m = await take(e, entry); }
      catch (err) {
        // A fetch inside failed before the entry was read: back off and read it again.
        if (run === chRun) ui.st.textContent = `Cannot reach ${rec.relay}. Retrying.`;
        failed = true; break;
      }
      rec.seq = entry.seq;
      if (m) { await saveMsg(m); if (run === chRun) ui.append(m); }
    }
    if (entries.length) await persist(e);
    if (failed) { await sleep(3000); continue; }
    if (!entries.length) synced = true;
    if (!synced || run !== chRun) continue;
    if (!e.owner) {
      if (e.retryEpoch) { try { if (!(await fetchEpoch(e, e.retryEpoch))) e.behind = true; e.retryEpoch = null; } catch { await sleep(3000); continue; } }
      // Review focus 1: a link with no deposit at the channel's current keys reads nothing new.
      const current = epochNumber(e.member.current_epoch_hex());
      if (e.behind || !current || (e.maxEpoch || 0) > current) { await markDead(e); ui.refresh(); return; }
      await askToJoin(e, ui);
    } else {
      try { await flushRetires(e); } catch { /* the next round tries again */ }
      try { if (await autoAllow(e)) U.toast('Let in a request the policy permits.', 'ok'); } catch { /* the next round tries again */ }
    }
  }
}
/** Post a join request once, then look for the grant on each round. */
async function askToJoin(e, ui) {
  const { rec } = e;
  // Until the owner lets this member post: a grant to read is an answer, but the owner may still
  // allow posting from the same request.
  if ((rec.grantChain && e.member.may_post()) || !rec.wantsPost) return;
  try {
    if (!rec.requested) {
      const actions = e.member.may_ask_to_post() ? 'read,post' : 'read';
      await chPost(rec.relay, `/v1/channels/${rec.id}/requests`, e.member.join_request(rec.myName || 'Someone', actions));
      rec.requested = true; await persist(e); ui.refresh();
      return;
    }
    const auth = readAuth(e) || { member: e.member };
    const j = await chGet(rec.relay, `/v1/channels/${rec.id}/grants?kid=${rec.kid}`, auth);
    // The newest grant that lets this member post, else the newest one (to read).
    const chains = (j.grants || []).map((g) => b64_decode(g));
    let chosen = null;
    for (const chain of [...chains].reverse()) { e.member.set_grant(chain); if (e.member.may_post()) { chosen = chain; break; } }
    chosen = chosen || chains[chains.length - 1] || null;
    const same = (x, y) => !!x && !!y && x.length === y.length && x.every((b, i) => b === y[i]);
    if (chosen && !same(chosen, rec.grantChain)) {
      e.member.set_grant(chosen);
      rec.grantChain = chosen;
      await persist(e); ui.refresh();
      U.toast(e.member.may_post() ? 'The owner let you post.' : 'The owner let you read, not post.', 'ok');
    } else if (rec.grantChain) e.member.set_grant(rec.grantChain);
  } catch (err) { if (err.status !== 401) ui.st.textContent = friendly(err); }
}

/** Make a new link (the policy's link evidence is the current link, so no policy change), register
 * its gate token, and start keys that carry a deposit for it. */
async function issueLink(e) {
  e.owner.link_url(e.rec.relay);
  await registerGate(e);
}
async function shareFlow(e) {
  try {
    if (!e.owner.gate_token_hash_hex()) {
      if (e.rec.preset === 4) { screenShare(e); return; }
      if (!confirm('This channel has no live link. Make a new one? Everyone you send it to can read.')) return;
      await issueLink(e);
      await startNewKeys(e);
    }
    screenShare(e);
  } catch (x) { U.toast(friendly(x), 'bad'); }
}

function policySheet(e) {
  const { h, seg, btn, status, stepper, field } = U;
  let preset = String(e.rec.preset ?? 0), quorum = e.rec.quorum || 2;
  const err = status();
  const q = field('Approvals needed', stepper(1, 8, quorum, (v) => { quorum = v; }), 'Seats of your circle that approve each post.');
  q.hidden = preset !== '1';
  const hint = h('span', { class: 'hint' }, PRESETS[Number(preset)][2]);
  U.sheet((close) => h('div', { class: 'stack' },
    h('h2', {}, 'Policy'), h('p', { class: 'muted' }, `Now: ${e.rec.policyLine}.`),
    h('div', { class: 'ch-presets' }, seg(PRESETS.map(([v, l]) => [v, l]), preset, (v) => { preset = v; q.hidden = v !== '1'; hint.textContent = PRESETS[Number(v)][2]; })), hint, q, err,
    btn('Save the policy', async () => {
      try {
        const body = e.owner.set_policy(Number(preset), preset === '1' ? quorum : 0);
        e.rec.preset = Number(preset); e.rec.quorum = preset === '1' ? quorum : 0;
        await publishPolicy(e, body);
        if (await registerGate(e)) await startNewKeys(e);
        close(); U.toast('Policy saved.', 'ok'); screenChannel(e.rec.id);
      } catch (x) { err.textContent = friendly(x); }
    }, 'primary')));
}

/** An announcement that needs approvals: propose it, show its code, collect approvals, post. */
async function proposeSheet(e, text, onSent, append) {
  const { h, btn, status } = U;
  const err = status();
  const pt = chat_bucket_pad(U.utf8.encode(text));
  if (!pt) { U.toast('Message too long.', 'bad'); return; }
  try { e.member.propose_announcement(pt); } catch (x) { U.toast(friendly(x), 'bad'); return; }
  const digest = e.member.announcement_digest();
  const firstLine = text.split('\n')[0].slice(0, 120);
  const log = BigInt(e.rec.seq ?? 0);
  const mine = e.owner.approve_post(digest, e.rec.name, firstLine, log);
  const code = `${digest}.${b64_encode(U.utf8.encode(firstLine))}.${log}`;
  const need = approvalsNeeded(e);
  const ta = h('textarea', { class: 'input', placeholder: 'Paste approvals here, one per line' });
  const count = h('p', { class: 'muted' }, `1 of ${need} approvals`);
  const approvals = () => [mine, ...ta.value.split(/\s+/).filter(Boolean).map((x) => b64_decode(x))];
  await persist(e); // the spent counter is recorded before anything leaves
  const send = async (close) => {
    try {
      const wire = e.member.send_proposed(cborArray(approvals()));
      await persist(e);
      const res = await postMessage(e.rec.relay, e.rec.id, b64_decode(wire));
      const m = { channel: e.rec.id, dir: 'out', kid: e.rec.kid, name: 'you', text, type: 'announcement', ts: Date.now(), seq: res?.seq ?? null, ok: true };
      await saveMsg(m); append(m); onSent(); close?.();
    } catch (x) { err.textContent = `Not sent: ${friendly(x)}`; if (!close) U.toast(err.textContent, 'bad'); }
  };
  if (need <= 1) { await send(); return; }
  U.sheet((close) => {
    const post = btn('Post', () => send(close), 'primary');
    post.disabled = true;
    ta.oninput = () => { let n = 1; try { n = approvals().length; } catch { /* not yet */ } count.textContent = `${n} of ${need} approvals`; post.disabled = n < need; };
    return h('div', { class: 'stack' },
      h('h2', {}, 'Ask for approvals'),
      h('p', { class: 'muted' }, `This channel needs ${need} approvals to post. Send this code to the other seats of your circle. Each opens Approve a post from the channel menu and sends back an approval.`),
      h('div', { class: 'ch-code' }, code), U.copyBtn(code, 'Copy the code'),
      ta, count, err, post);
  });
}
function approveSheet(e, circle = null) {
  const { h, btn, status } = U;
  const err = status();
  const ta = h('textarea', { class: 'input', placeholder: 'Paste the code from the person posting' });
  const out = h('div', { class: 'stack' });
  U.sheet(() => h('div', { class: 'stack' },
    h('h2', {}, 'Approve a post'), h('p', { class: 'muted' }, 'Your seat signs that it approves this one post.'), ta,
    btn('Read the code', () => {
      try {
        const [digest, line, log] = ta.value.trim().split('.');
        if (!/^[0-9a-f]{64}$/.test(digest || '')) throw 'digest';
        const firstLine = U.utf8d.decode(b64_decode(line || ''));
        out.replaceChildren(
          h('p', {}, h('b', {}, 'First line: '), firstLine), h('p', { class: 'muted ch-code' }, digest),
          btn('Approve', () => {
            try {
              const approval = e.owner ? e.owner.approve_post(digest, e.rec.name, firstLine, BigInt(log || 0)) : circle.approve_channel_post(e.rec.policy, digest, e.rec.name, firstLine, BigInt(log || 0));
              const text = b64_encode(approval);
              out.replaceChildren(h('p', { class: 'muted' }, 'Send this approval back to the person posting.'), h('div', { class: 'ch-code' }, text), U.copyBtn(text, 'Copy the approval'));
            } catch (x) { err.textContent = friendly(x); }
          }, 'primary'));
      } catch (x) { err.textContent = friendly(x); }
    }), out, err));
}

async function screenRequests(id) {
  const { h, btn, status, screen } = U;
  const generation = U.generation();
  const e = await openChannel(id);
  if (!e || !e.owner) return U.go(`#/ch/${id}`);
  const settings = await loadChSettings(id);
  const st = status();
  const listEl = h('div', { class: 'rows' }, h('p', { class: 'muted' }, 'Reading requests…'));
  const render = async () => {
    let reqs;
    try { reqs = await fetchRequests(e); } catch (x) { st.textContent = friendly(x); return; }
    if (generation !== U.generation()) return;
    if (!reqs.length) { listEl.replaceChildren(h('p', { class: 'muted' }, 'No requests are waiting.')); return; }
    listEl.replaceChildren(...reqs.map((r) => {
      const bytes = b64_decode(r.body);
      let s;
      try { s = JSON.parse(join_request_summary(bytes)); } catch { s = null; }
      const refuse = btn('Refuse', async () => { await markHandled(e, r.id); render(); });
      if (!s || s.channel_id !== id) return h('div', { class: 'request' }, h('div', {}, h('b', {}, 'A request that does not check out')), refuse);
      // The owner picks what to give: read, or read and post. Admit and govern are never given.
      const choice = (label, actions, kind) => {
        const b = btn(label, async () => {
          b.disabled = true;
          // A request given less than it asked stays listed, so posting can be allowed later.
          try { const given = await admitRequest(e, bytes, actions); if (s.actions.split(',').every((a) => given.split(',').includes(a.trim()))) await markHandled(e, r.id); U.toast(`${s.name} can ${given.replace(',', ' and ')}.`, 'ok'); render(); }
          catch (x) { st.textContent = friendly(x); b.disabled = false; }
        }, kind);
        return b;
      };
      const asksToPost = s.actions.split(',').includes('post');
      const allows = asksToPost ? [choice('Allow to post', 'read,post', 'primary'), choice('Allow to read', 'read', '')] : [choice('Allow to read', 'read', 'primary')];
      return h('div', { class: 'request' }, h('div', {}, h('b', {}, s.name), h('span', {}, `asks to ${s.actions.replace(/,/g, ', ')} · key ${short(s.kid)}`)), h('div', { class: 'request-actions' }, ...allows, refuse));
    }));
  };
  screen({
    title: 'Requests', sub: e.rec.name, back: `#/ch/${id}`,
    body: [h('div', { class: 'stack' },
      h('h1', {}, 'Who wants to post.'),
      h('p', { class: 'lead' }, 'Each person typed their own name. Allow gives them a grant to post; you can revoke it from Members.'),
      listEl,
      h('label', { class: 'toggle' }, h('span', {}, h('b', {}, 'Allow automatically what the policy already permits'), h('br'), h('span', { class: 'muted' }, 'Requests that came with the live link are let in while this channel is open here.')),
        h('input', { type: 'checkbox', checked: !!settings.autoAllow, onchange: async (ev) => { await saveChSettings({ ...settings, autoAllow: ev.target.checked }); } })),
      st)],
  });
  render();
}

async function screenMembers(id) {
  const { h, btn, status, screen } = U;
  const e = await openChannel(id);
  if (!e || !e.owner) return U.go(`#/ch/${id}`);
  const st = status();
  const members = JSON.parse(e.owner.members_json());
  const revoke = async (m, button) => {
    const holdsLink = !!e.owner.gate_token_hash_hex();
    if (!confirm(`Revoke ${m.name}? ${holdsLink ? 'They hold the link, so it is replaced by a new one, and ' : ''}new keys start.${holdsLink ? ' Everyone who should keep reading needs the new link.' : ''}`)) return;
    button.disabled = true;
    try {
      const [statement, control] = cbor_split(e.owner.revoke(m.grant_id, 'removed by the owner'));
      await persist(e);
      await chPost(e.rec.relay, `/v1/channels/${id}/revocations`, b64_decode(statement));
      await postMessage(e.rec.relay, id, b64_decode(control));
      // Retire, a new link and its gate, then keys whose deposits include the new link's.
      if (holdsLink) { await retireLink(e); await issueLink(e); }
      await startNewKeys(e);
      U.toast(`${m.name} is removed and new keys started.`, 'ok');
      if (holdsLink) screenShare(e, { note: NEW_LINK }); else screenMembers(id);
    } catch (x) { st.textContent = friendly(x); button.disabled = false; }
  };
  screen({
    title: 'Members', sub: e.rec.name, back: `#/ch/${id}`,
    body: [h('div', { class: 'stack' },
      h('h1', {}, 'People you let in.'),
      h('p', { class: 'lead' }, 'Each holds a grant from your seat. Revoking one ends it and starts new keys.'),
      members.length ? h('div', { class: 'rows' }, ...members.map((m) => {
        const b = btn('Revoke', () => revoke(m, b), 'danger');
        return h('div', { class: 'member' }, h('div', {}, h('b', {}, m.name), h('span', {}, `${(e.rec.actions?.[m.grant_id] || 'read').replace(',', ', ')} · no expiry · key ${short(m.kid)}`)), b);
      })) : h('p', { class: 'muted' }, 'No one else is a member yet.'),
      st)],
  });
}
