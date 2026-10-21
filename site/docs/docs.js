// Renders the repository's markdown docs. The deploy workflow copies docs/*.md and
// selected crate READMEs into site/docs/md/. Routes are #/<name>.
const DOCS = [
  { group: "Start here" },
  { id: "readme", title: "Overview", file: "readme.md" },
  { id: "use-cases", title: "What it is for", file: "use-cases.md" },
  { id: "tiers", title: "Device tiers", file: "tiers.md" },
  { id: "ceremony-guide", title: "Ceremony guide", file: "ceremony-guide.md" },
  { group: "Specification" },
  { id: "protocol", title: "Protocol v1", file: "protocol.md" },
  { id: "chat", title: "Pad-seeded chat", file: "chat.md" },
  { id: "threat-model", title: "Threat model", file: "threat-model.md" },
  { group: "Project" },
  { id: "plan", title: "Plan and milestones", file: "plan.md" },
  { group: "Crates" },
  { id: "web", title: "Browser app", file: "web.md" },
  { id: "station", title: "Numbers Station relay", file: "station.md" },
  { id: "chat-cli", title: "Chat CLI demo", file: "chat-cli.md" },
  { id: "host", title: "Host simulator", file: "host.md" },
  { id: "yk", title: "YubiKey entropy", file: "yk.md" },
  { id: "adapters", title: "Adapters", file: "adapters.md" },
];

const nav = document.getElementById("nav");
const content = document.getElementById("content");

for (const d of DOCS) {
  if (d.group) {
    const g = document.createElement("div");
    g.className = "group";
    g.textContent = d.group;
    nav.appendChild(g);
    continue;
  }
  const a = document.createElement("a");
  a.href = `#/${d.id}`;
  a.textContent = d.title;
  a.dataset.id = d.id;
  nav.appendChild(a);
}

function current() {
  const m = location.hash.match(/^#\/([a-z0-9-]+)/);
  const id = m ? m[1] : "readme";
  return DOCS.find((d) => d.id === id) || DOCS[1];
}

// Links inside the markdown point at repo paths like docs/protocol.md. Map them to routes.
function rewriteLinks(root) {
  for (const a of root.querySelectorAll("a[href]")) {
    const href = a.getAttribute("href");
    const m = href.match(/(?:^|\/)([a-z0-9-]+)\.md(#.*)?$/i);
    if (m) {
      const d = DOCS.find((x) => x.file === `${m[1].toLowerCase()}.md`);
      if (d) a.setAttribute("href", `#/${d.id}`);
      else a.setAttribute("href", `https://github.com/yourbuddyconner/cahoots/blob/main/${href}`);
    } else if (/^https?:/.test(href)) {
      a.setAttribute("rel", "noopener");
    }
  }
}

async function render() {
  const d = current();
  for (const a of nav.querySelectorAll("a")) a.classList.toggle("on", a.dataset.id === d.id);
  document.title = `${d.title} · Cahoots docs`;
  content.innerHTML = '<p class="loading">Loading…</p>';
  try {
    const r = await fetch(`md/${d.file}`, { cache: "no-cache" });
    if (!r.ok) throw new Error(`${r.status}`);
    const md = await r.text();
    content.innerHTML = marked.parse(md, { gfm: true });
    rewriteLinks(content);
    if (location.hash.includes("#", 2)) {
      const sub = location.hash.slice(location.hash.indexOf("#", 2) + 1);
      document.getElementById(sub)?.scrollIntoView();
    } else {
      scrollTo(0, 0);
    }
  } catch (e) {
    content.innerHTML = `<p>This document isn't published yet (${e.message}). It lives in the repository under <code>docs/</code>.</p>`;
  }
}
addEventListener("hashchange", render);
render();
