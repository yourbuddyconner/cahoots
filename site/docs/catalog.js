// The sidebar is a reading path. Longer lab notes remain available by direct URL.
export const DOCS = [
  { group: 'Start here' },
  { id: 'readme', title: 'Start here', file: 'overview.md', src: 'docs/overview.md' },
  { id: 'apps', title: 'Apps for your circle', file: 'apps.md', src: 'docs/apps.md' },
  { id: 'ceremony-guide', title: 'Create a circle', file: 'ceremony-guide.md', src: 'docs/ceremony-guide.md' },
  { id: 'web', title: 'Use the browser app', file: 'browser.md', src: 'docs/browser.md' },
  { id: 'status', title: 'Project status', file: 'status.md', src: 'docs/status.md' },
  { group: 'Guides' },
  { id: 'station', title: 'Run a relay', file: 'relay.md', src: 'docs/relay.md' },
  { id: 'hardware', title: 'Hardware', file: 'hardware.md', src: 'docs/hardware.md' },
  { id: 'cahoots', title: 'Cahoots, the faces', file: 'cahoots.md', src: 'docs/cahoots.md' },
  { id: 'security', title: 'Security and limits', file: 'security.md', src: 'docs/security.md' },
  { group: 'Reference' },
  { id: 'glossary', title: 'Glossary', file: 'glossary.md', src: 'docs/glossary.md' },
  { id: 'tiers', title: 'Device tiers', file: 'tiers.md', src: 'docs/tiers.md' },
  { id: 'adapters', title: 'Integrations', file: 'adapters.md', src: 'crates/ADAPTERS.md' },
  { id: 'protocol', title: 'Protocol specification', file: 'protocol.md', src: 'docs/protocol.md', spec: true },
  { id: 'chat', title: 'Chat specification', file: 'chat.md', src: 'docs/chat.md', spec: true },
];

// Keep existing bookmarks and in-document links working without putting every
// research note or implementation README in the main navigation.
export const ARCHIVED = [
  { id: 'circle-apps', title: 'Circle-app proposal', file: 'circle-apps.md', src: 'docs/circle-apps.md', category: 'Design notes' },
  { id: 'use-cases', title: 'Uses for a circle', file: 'use-cases.md', src: 'docs/use-cases.md', category: 'Guides' },
  { id: 'threat-model', title: 'Threat model', file: 'threat-model.md', src: 'docs/threat-model.md', category: 'Reference' },
  { id: 'diy-badge', title: 'DIY wiring and bring-up', file: 'diy-badge.md', src: 'docs/diy-badge.md', category: 'Hardware reference' },
  { id: 'v1-board', title: 'V1 board reference', file: 'v1-board.md', src: 'hardware/v1/README.md', category: 'Hardware reference' },
  { id: 'rp2350', title: 'Pico 2 firmware', file: 'rp2350.md', src: 'crates/rp2350/README.md', category: 'Hardware reference' },
  { id: 'manufacturing', title: 'Manufacturing notes', file: 'manufacturing.md', src: 'docs/manufacturing.md', category: 'Development notes' },
  { id: 'gatherings', title: 'Gathering concepts', file: 'gatherings.md', src: 'docs/gatherings.md', category: 'Design notes' },
  { id: 'attested-data', title: 'Attested sensor concepts', file: 'attested-data.md', src: 'docs/attested-data.md', category: 'Design notes' },
  { id: 'plan', title: 'Development roadmap', file: 'plan.md', src: 'docs/plan.md', category: 'Development notes' },
  { id: 'chat-cli', title: 'Chat CLI', file: 'chat-cli.md', src: 'crates/chat/README.md', category: 'Developer reference' },
  { id: 'host', title: 'Badge simulator', file: 'host.md', src: 'crates/host/README.md', category: 'Developer reference' },
];
