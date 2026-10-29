// Load the optional 3D enhancement only when the hardware section approaches view.
// The board image remains visible if loading or WebGL initialization fails.
const stage = document.getElementById('badge-stage');
if (stage) {
  const observer = new IntersectionObserver(async ([entry]) => {
    if (!entry.isIntersecting) return;
    observer.disconnect();
    try {
      const { initBadge } = await import('./badge-render.js');
      await initBadge(stage);
    } catch (error) {
      stage.querySelector('canvas').hidden = true;
      stage.querySelector('.badge-fallback').hidden = false;
      stage.querySelector('button').hidden = true;
      console.warn('Badge preview unavailable; showing the board image.', error);
    }
  }, { rootMargin: '200px' });
  observer.observe(stage);
}
