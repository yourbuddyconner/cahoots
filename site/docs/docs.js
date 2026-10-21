// Renders the repository's markdown docs. The deploy workflow copies docs/*.md and
// selected crate READMEs into site/docs/md/, and docs/diagrams into site/docs/diagrams/.
// Routes are #/<name>, with #/<name>#<heading> for a section.
const DOCS = [
  { group: "Start here" },
  { id: "readme", title: "Overview", file: "readme.md", lede: "What Cahoots is and how the pieces fit." },
  { id: "use-cases", title: "What it is for", file: "use-cases.md", lede: "Concrete uses, from group chat to bootstrapping a network." },
  { id: "ceremony-guide", title: "Ceremony guide", file: "ceremony-guide.md", lede: "How to run a ceremony with a group." },
  { id: "tiers", title: "Device tiers", file: "tiers.md", lede: "What each kind of device protects, from a phone to inspected silicon." },
  { id: "diy-badge", title: "Build a badge", file: "diy-badge.md", lede: "Two hardware families: the Baochip badge and the Pico badge. E-paper, two buttons, a wire, USB to the phone." },
  { id: "glossary", title: "Glossary", file: "glossary.md", lede: "The words these pages use, in plain terms." },
  { group: "Specification" },
  { id: "protocol", title: "Protocol v1", file: "protocol.md", lede: "The normative description of the ceremony, messages, and storage.", spec: true },
  { id: "chat", title: "Chat", file: "chat.md", lede: "Keys from the pad, forward secrecy per sender, messages over the feed.", spec: true },
  { id: "threat-model", title: "Threat model", file: "threat-model.md", lede: "What is covered, what is not, and what the app may claim." },
  { group: "Project" },
  { id: "plan", title: "Roadmap", file: "plan.md", lede: "What is built, what is next, and what is waiting." },
  { group: "Reference" },
  { id: "web", title: "Browser app", file: "web.md", lede: "Running and building the app." },
  { id: "station", title: "The feed", file: "station.md", lede: "Running the relay." },
  { id: "chat-cli", title: "Chat CLI", file: "chat-cli.md", lede: "A terminal chat client for trying the protocol." },
  { id: "host", title: "Simulator", file: "host.md", lede: "A desktop stand-in for the badge." },
  { id: "yk", title: "YubiKey entropy", file: "yk.md", lede: "Randomness from a YubiKey." },
  { id: "adapters", title: "Adapters", file: "adapters.md", lede: "HPKE, age, and WireGuard." },
];
const PAGES = DOCS.filter((d) => d.id);

const nav = document.getElementById("nav");
const content = document.getElementById("content");
const toc = document.getElementById("toc");
const drawer = document.getElementById("drawer");
const drawerLabel = document.getElementById("drawer-label");

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
// The mobile drawer gets its own copy of the document list.
const navCopy = nav.cloneNode(true);
navCopy.id = "";
drawer.appendChild(navCopy);

function current() {
  const m = location.hash.match(/^#\/([a-z0-9-]+)/);
  const id = m ? m[1] : "readme";
  return PAGES.find((d) => d.id === id) || PAGES[0];
}

function groupOf(d) {
  let g = "";
  for (const x of DOCS) {
    if (x.group) g = x.group;
    if (x === d) return g;
  }
  return g;
}

function slug(s) {
  return s.toLowerCase().replace(/[^\w\s-]/g, "").trim().replace(/\s+/g, "-");
}

// Links inside the markdown point at repo paths like docs/protocol.md. Map them to routes.
function rewriteLinks(root) {
  for (const a of root.querySelectorAll("a[href]")) {
    const href = a.getAttribute("href");
    const m = href.match(/(?:^|\/)([a-z0-9-]+)\.md(#.*)?$/i);
    if (m) {
      const d = PAGES.find((x) => x.file === `${m[1].toLowerCase()}.md`);
      if (d) a.setAttribute("href", `#/${d.id}${m[2] || ""}`);
      else a.setAttribute("href", `https://github.com/yourbuddyconner/cahoots/blob/main/${href}`);
    } else if (/^https?:/.test(href)) {
      a.setAttribute("rel", "noopener");
    }
  }
}

// Give headings ids and anchor links, and build the on-page contents.
function headings(root) {
  const items = [];
  const seen = new Set();
  for (const h of root.querySelectorAll("h2, h3")) {
    let id = slug(h.textContent);
    while (seen.has(id)) id += "-";
    seen.add(id);
    h.id = id;
    const a = document.createElement("a");
    a.className = "anchor";
    a.href = `#/${current().id}#${id}`;
    a.setAttribute("aria-label", "Link to this section");
    a.textContent = "#";
    h.appendChild(a);
    items.push({ id, level: h.tagName === "H2" ? 2 : 3, text: h.firstChild.textContent.trim() });
  }
  toc.innerHTML = "";
  if (items.length < 2) {
    toc.hidden = true;
    return;
  }
  toc.hidden = false;
  const t = document.createElement("p");
  t.className = "toc-title";
  t.textContent = "On this page";
  toc.appendChild(t);
  for (const it of items) {
    const a = document.createElement("a");
    a.href = `#/${current().id}#${it.id}`;
    a.textContent = it.text;
    a.className = `l${it.level}`;
    a.dataset.id = it.id;
    toc.appendChild(a);
  }
}

// Wrap tables so wide ones scroll instead of breaking the layout. Inline SVG
// diagrams so they use the page fonts.
async function figures(root) {
  for (const t of root.querySelectorAll("table")) {
    const w = document.createElement("div");
    w.className = "table-wrap";
    t.replaceWith(w);
    w.appendChild(t);
  }
  for (const img of root.querySelectorAll("img[src$='.svg']")) {
    const fig = document.createElement("figure");
    fig.className = "diagram";
    img.replaceWith(fig);
    // The markdown refers to the diagrams by repo path. They are served next to this page.
    const src = `diagrams/${img.getAttribute("src").split("/").pop()}`;
    try {
      const r = await fetch(src);
      if (!r.ok) throw new Error(r.status);
      fig.innerHTML = await r.text();
      const svg = fig.querySelector("svg");
      svg.removeAttribute("width");
      svg.removeAttribute("height");
    } catch {
      fig.appendChild(img);
    }
    if (img.alt) {
      const cap = document.createElement("figcaption");
      cap.textContent = img.alt;
      fig.appendChild(cap);
    }
  }
}

function pager(d) {
  const i = PAGES.indexOf(d);
  const prev = PAGES[i - 1];
  const next = PAGES[i + 1];
  const p = document.createElement("nav");
  p.className = "pager";
  p.setAttribute("aria-label", "Previous and next");
  p.innerHTML =
    (prev ? `<a class="prev" href="#/${prev.id}"><span>Previous</span>${prev.title}</a>` : "<span></span>") +
    (next ? `<a class="next" href="#/${next.id}"><span>Next</span>${next.title}</a>` : "<span></span>");
  return p;
}

function watchHeadings() {
  const hs = [...content.querySelectorAll("h2, h3")];
  if (!hs.length || !("IntersectionObserver" in window)) return;
  const on = (id) => {
    for (const a of toc.querySelectorAll("a")) a.classList.toggle("on", a.dataset.id === id);
  };
  const io = new IntersectionObserver(
    (entries) => {
      const vis = entries.filter((e) => e.isIntersecting).sort((a, b) => a.boundingClientRect.top - b.boundingClientRect.top);
      if (vis.length) on(vis[0].target.id);
    },
    { rootMargin: "-80px 0px -70% 0px" }
  );
  hs.forEach((h) => io.observe(h));
}

let lastId = null;
async function render() {
  const d = current();
  const sub = location.hash.includes("#", 2) ? location.hash.slice(location.hash.indexOf("#", 2) + 1) : "";
  for (const a of document.querySelectorAll(".docs-nav a")) a.classList.toggle("on", a.dataset.id === d.id);
  document.title = `${d.title} · Cahoots docs`;
  drawerLabel.textContent = d.title;
  drawer.open = false;
  if (d.id !== lastId) {
    lastId = d.id;
    content.innerHTML = '<p class="loading">Loading…</p>';
    try {
      const r = await fetch(`md/${d.file}`, { cache: "no-cache" });
      if (!r.ok) throw new Error(`${r.status}`);
      const md = await r.text();
      content.innerHTML = marked.parse(md, { gfm: true });
      const h1 = content.querySelector("h1");
      const head = document.createElement("header");
      head.className = "doc-head";
      head.innerHTML = `<p class="eyebrow">${groupOf(d)}${d.spec ? " · normative" : ""}</p>`;
      if (h1) {
        h1.replaceWith(head);
        head.appendChild(h1);
      } else {
        content.prepend(head);
      }
      if (d.lede) {
        const l = document.createElement("p");
        l.className = "doc-lede";
        l.textContent = d.lede;
        head.appendChild(l);
      }
      rewriteLinks(content);
      headings(content);
      await figures(content);
      content.appendChild(pager(d));
      watchHeadings();
    } catch (e) {
      content.innerHTML = `<p>This document isn't published yet (${e.message}). It lives in the repository under <code>docs/</code>.</p>`;
    }
  }
  if (sub) {
    document.getElementById(sub)?.scrollIntoView({ block: "start" });
  } else {
    scrollTo(0, 0);
  }
}
addEventListener("hashchange", render);
render();
