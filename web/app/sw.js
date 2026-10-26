// Network-first service worker with a cache fallback, so a load with a connection always
// gets the current app, and the app still opens with no connection. Bump VERSION on release.
const VERSION = 'cahoots-v13';
const ASSETS = [
  './', './index.html', './styles.css', './app.js', './manifest.json', './icon.svg',
  './pkg/cahoots_wasm.js', './pkg/cahoots_wasm_bg.wasm',
];
self.addEventListener('install', (e) => {
  e.waitUntil(caches.open(VERSION).then((c) => c.addAll(ASSETS)).then(() => self.skipWaiting()));
});
self.addEventListener('activate', (e) => {
  e.waitUntil(caches.keys().then((keys) => Promise.all(keys.filter((k) => k !== VERSION).map((k) => caches.delete(k)))).then(() => self.clients.claim()));
});
self.addEventListener('fetch', (e) => {
  if (e.request.method !== 'GET') return;
  const url = new URL(e.request.url);
  // Relay traffic is network-only. Never cache ciphertext feeds.
  if (url.pathname.includes('/v1/')) return;
  if (url.origin !== location.origin) return;
  e.respondWith(
    fetch(e.request)
      .then((res) => {
        if (res.ok) caches.open(VERSION).then((c) => c.put(e.request, res.clone())).catch(() => {});
        return res;
      })
      .catch(() => caches.match(e.request, { ignoreSearch: true })),
  );
});
