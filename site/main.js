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
  const lines = [];
  for (let r = 0; r < 6; r++) lines.push(groups(8).join(" "));
  sample.textContent = lines.join("\n");
}
