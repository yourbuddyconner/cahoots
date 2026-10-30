# Circle workspace implementation plan

**Goal:** Preserve the app facelift while making each circle a usable workspace on phone and desktop.

**Architecture:** Keep the existing protocol, IndexedDB, relay and ceremony routes. A shared circle shell contains a switcher and contextual tool navigation. The circle opens directly into its live conversation, with people beside it on desktop. Mobile uses a compact section row and tools sheet. People and Network render inside the same shell. The entry screen remains a simple circle list with create/join actions. The marketing homepage is unchanged.

**Networking reference:** Use a restrained administrative layout like the provided screenshot: persistent workspace navigation, a clear page header and table, and contextual setup guidance. No invented peer records, addresses or live tunnel controls; the CLI integration is documented and not connected to browser state.

**Files:** `web/app/app.js` for shell/views and per-circle in-memory drafts; `styles.css` for desktop/mobile layouts; app metadata/icons/service worker for the retained light theme; `web/e2e.mjs` for behavioral and visual checks.

1. Add checks that a saved circle opens into its composer, without a second Chat launch or an Explore split.
2. Build one persistent shell for Conversation, People and Network. Keep focused QR/note workflows accessible through contextual tools.
3. Preserve unsent drafts when switching tools and resizing. Preserve local nicknames when changing relay configuration.
4. Verify desktop rail/people context, mobile controls and composer, Network's honest setup state, and local member names.
5. Re-run two/three-person ceremonies, rendered QR decoding, compact QR/camera layouts, chat delivery, short notes and replay rejection. Review actual desktop/mobile screenshots before reporting completion.
