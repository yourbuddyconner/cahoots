// Reading aids for journal posts. Definitions also work through CSS without JavaScript.
(() => {
  const article = document.querySelector('.post');
  if (!article) return;
  document.body.dataset.readingEnhanced = 'true';
  const progress = document.querySelector('.reading-progress');
  let scheduled = false;
  const measure = () => {
    scheduled = false;
    if (!progress) return;
    const bounds = article.getBoundingClientRect();
    const header = document.querySelector('.top')?.getBoundingClientRect().height || 0;
    const start = bounds.top + scrollY - header;
    const end = bounds.bottom + scrollY - innerHeight;
    const fraction = end <= start ? (scrollY >= start ? 1 : 0) : Math.max(0, Math.min(1, (scrollY - start) / (end - start)));
    progress.style.setProperty('--reading-fraction', fraction);
    progress.setAttribute('aria-valuenow', String(Math.round(fraction * 100)));
  };
  const schedule = () => {
    if (!scheduled) { scheduled = true; requestAnimationFrame(measure); }
  };
  if (progress) progress.hidden = false;
  addEventListener('scroll', schedule, { passive:true });
  addEventListener('resize', schedule);
  article.addEventListener('load', schedule, true);
  article.addEventListener('toggle', schedule, true);
  if ('ResizeObserver' in window) new ResizeObserver(schedule).observe(article);
  document.fonts?.ready.then(schedule);
  schedule();

  let active = null;
  let closeTimer = null;
  const cancelClose = () => { clearTimeout(closeTimer); closeTimer = null; };
  const close = () => {
    cancelClose();
    if (active) delete active.dataset.open;
    active = null;
  };
  const show = term => {
    cancelClose();
    if (active !== term) close();
    const tip = term.querySelector('.term-definition');
    const rect = term.getBoundingClientRect();
    tip.style.width = `${Math.min(300, innerWidth - 32)}px`;
    const height = tip.getBoundingClientRect().height;
    const width = tip.getBoundingClientRect().width;
    const left = Math.max(16, Math.min(innerWidth - width - 16, rect.left));
    const above = rect.top - height - 10;
    const top = above >= 16 ? above : Math.min(innerHeight - height - 16, rect.bottom + 10);
    tip.style.left = `${left}px`;
    tip.style.top = `${Math.max(16, top)}px`;
    active = term;
    term.dataset.open = '';
  };
  article.querySelectorAll('.defined-term').forEach(term => {
    const deferClose = () => {
      cancelClose();
      closeTimer = setTimeout(() => {
        if (active === term && document.activeElement !== term && !term.matches(':hover')) close();
      }, 180);
    };
    term.addEventListener('pointerenter', event => { if (event.pointerType !== 'touch') show(term); });
    term.addEventListener('pointerleave', deferClose);
    term.querySelector('.term-definition').addEventListener('pointerenter', cancelClose);
    term.querySelector('.term-definition').addEventListener('pointerleave', deferClose);
    term.addEventListener('focus', () => show(term));
    term.addEventListener('blur', close);
    term.addEventListener('click', () => show(term));
  });
  document.addEventListener('keydown', event => {
    if (event.key === 'Escape' && active) { close(); event.preventDefault(); }
  });
  document.addEventListener('pointerdown', event => { if (active && !active.contains(event.target)) close(); });
  addEventListener('scroll', () => {
    if (!active) return;
    const bounds = active.getBoundingClientRect();
    if (bounds.bottom < 0 || bounds.top > innerHeight ||
        (document.activeElement !== active && !active.matches(':hover'))) close();
    else show(active);
  }, { passive:true });
  addEventListener('resize', close);
  article.addEventListener('toggle', () => { close(); schedule(); }, true);
})();
