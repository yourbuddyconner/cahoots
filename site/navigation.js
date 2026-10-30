// Without JavaScript, the primary links remain visible at every screen size.
const header = document.querySelector('header.top');
const toggle = header.querySelector('.nav-toggle');
const navigation = document.getElementById(toggle.getAttribute('aria-controls'));
const mobile = window.matchMedia('(max-width: 720px)');

function setMenu(open, restoreFocus = false) {
  navigation.hidden = mobile.matches && !open;
  toggle.setAttribute('aria-expanded', String(open));
  toggle.setAttribute('aria-label', open ? 'Close navigation' : 'Open navigation');
  toggle.querySelector('.nav-toggle-label').textContent = open ? 'Close' : 'Menu';
  if (restoreFocus) toggle.focus();
}

function syncViewport() {
  const focusInNavigation = navigation.contains(document.activeElement);
  const focusOnToggle = document.activeElement === toggle;
  toggle.hidden = !mobile.matches;
  setMenu(false, mobile.matches && focusInNavigation);
  if (!mobile.matches && focusOnToggle) navigation.querySelector('a').focus();
}

toggle.addEventListener('click', () => {
  setMenu(toggle.getAttribute('aria-expanded') !== 'true');
});

navigation.addEventListener('click', (event) => {
  if (mobile.matches && event.target.closest('a')) setMenu(false);
});

document.addEventListener('click', (event) => {
  if (mobile.matches && !header.contains(event.target)) setMenu(false);
});

document.addEventListener('keydown', (event) => {
  if (event.key === 'Escape' && toggle.getAttribute('aria-expanded') === 'true') {
    setMenu(false, true);
  }
});

mobile.addEventListener('change', syncViewport);
syncViewport();
