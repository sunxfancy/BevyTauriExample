(() => {
  if (window.__desktopSmokeProbe) return;
  window.__desktopSmokeProbe = true;
  const errors = [];
  window.addEventListener('error', e => errors.push(String(e.message)));
  window.addEventListener('unhandledrejection', e => errors.push(String(e.reason)));
  const rect = selector => {
    const element = document.querySelector(selector);
    if (!element) return null;
    const r = element.getBoundingClientRect();
    return { x: r.x, y: r.y, width: r.width, height: r.height };
  };
  let sequence = 0;
  let pending = false;
  setInterval(async () => {
    if (pending) return;
    pending = true;
    try {
      await window.__TAURI_INTERNALS__.invoke('report', {state: {
        sequence: ++sequence, width: innerWidth, height: innerHeight,
        text: document.body.innerText, name: document.querySelector('#greet-input')?.value,
        input: rect('#greet-input'), button: rect('button[type=submit]'),
        logo: rect('img.react'), panel: rect('main'), greeting: rect('#greet-result'), errors,
      }});
    } catch (error) { console.error('CI probe:', error); }
    finally { pending = false; }
  }, 250);
})();
