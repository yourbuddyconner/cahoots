// Cahoots landing page: the Numbers Station ticker and the feed sample, both real random
// five-digit groups.
function groups(n) {
  const out = [];
  const buf = new Uint32Array(n);
  crypto.getRandomValues(buf);
  for (let i = 0; i < n; i++) out.push(String(buf[i] % 100000).padStart(5, "0"));
  return out;
}
const ticker = document.getElementById("ticker");
if (ticker) {
  const g = groups(120).join("  ");
  ticker.textContent = g + "    " + g;
}
const sample = document.getElementById("station-sample");
if (sample) {
  // One run of forty groups, wrapped by the card's width: eight to a row on a wide screen,
  // five on a phone, so no row is left half full.
  sample.textContent = groups(40).join(" ");
}
