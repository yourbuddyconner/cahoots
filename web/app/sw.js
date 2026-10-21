// Cache-first service worker. Bump VERSION when any asset changes.
const VERSION = 'cahoots-v1';
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
  e.respondWith(caches.match(e.request, { ignoreSearch: true }).then((hit) => hit || fetch(e.request)));
});
