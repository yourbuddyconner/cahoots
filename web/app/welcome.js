// First-use guidance. Reopening the guide never changes the route the person was on.
export function showWelcome({ h, screen, btn, link }, done, firstVisit = true) {
  const pages = [
    {
      title: 'A little circle of your people',
      detail: 'Meet in person to create a private circle. Then keep talking, wherever you go. No account or phone number needed.',
      topics: [
        ['Set up together', 'One person creates the circle. Everyone exchanges codes and checks the same six words. We call this setup a ceremony.'],
        ['Give everyone a name', 'Each device gets a numbered seat. In People, add familiar names that only you see.'],
      ],
    },
    {
      title: 'A familiar voice, a little more certainty',
      detail: 'Hello words help you check someone on a call using a secret shared by your two devices.',
      topics: [
        ['Open each other’s seat', 'In the same circle, open Hello words and select the person you’re talking to. Make sure both devices show the same hello number.'],
        ['Take turns saying two words', 'Read your own pair aloud and check their reply. Don’t tell them the words you expect to hear. When both pairs match, each of you selects Next hello.'],
      ],
    },
    {
      title: 'Your circle can reach a little further',
      detail: 'A channel lets your circle share messages with a wider audience. Choose who can read and who can post.',
      topics: [
        ['Share a link thoughtfully', 'Anyone with a channel link can read and forward it. Choose Closed for a private channel without a shared link.'],
        ['You control posting', 'A link doesn’t automatically let someone post. Review requests in the channel’s menu. You can remove a member’s future access later.'],
      ],
    },
    {
      title: 'Yours to keep. Here in this browser.',
      detail: 'Cahoots stores your keys and history in this browser. There is no account to sign back into or restore them from.',
      topics: [
        ['Keep your browser data', 'Clearing site data or using a different browser can mean losing access to your circles. To use another device, create a new circle together. A channel reader can reopen the current channel link.'],
        ['Know what leaves your device', 'A relay delivers encrypted messages without being able to read them. Messages already received can be saved by the people who read them.'],
      ],
    },
  ];
  let index = 0;
  const render = () => {
    const page = pages[index];
    const heading = h('h1', { tabindex: '-1', 'data-welcome-page': index }, page.title);
    screen({
      title: firstVisit ? 'Welcome to Cahoots' : 'How Cahoots works',
      sub: `${index + 1} of ${pages.length}`,
      back: index > 0 ? () => { index--; render(); } : null,
      body: [h('div', { class: 'stack' },
        heading,
        h('p', { class: 'lead' }, page.detail),
        ...page.topics.map(([title, detail], topic) => h('section', { class: 'stack' },
          h('h2', {}, `${topic + 1}. ${title}`), h('p', { class: 'muted' }, detail))),
        h('p', { class: 'hint' }, 'You can reopen this guide from the home screen’s More options menu.'))],
      actions: [
        btn(index === pages.length - 1 ? 'Get started' : 'Continue', () => {
          if (index === pages.length - 1) done(); else { index++; render(); }
        }, 'primary'),
        link(firstVisit ? 'Skip for now' : 'Close guide', done),
      ],
    }).classList.add('welcome-screen');
    heading.focus({ preventScroll: true });
  };
  render();
}
