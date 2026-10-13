(() => {
  const r = e => { const b = e.getBoundingClientRect(); return [+b.x.toFixed(1), +b.y.toFixed(1), +b.width.toFixed(1), +b.height.toFixed(1)]; };
  const vis = e => e && e.offsetParent !== null && e.getBoundingClientRect().width > 0;
  const out = {};
  const pick = (name, sel, n = 1) => {
    const els = [...document.querySelectorAll(sel)].filter(vis).slice(0, n);
    out[name] = els.map(e => {
      const s = getComputedStyle(e);
      return { r: r(e), fs: s.fontSize, fw: s.fontWeight, lh: s.lineHeight, c: s.color, bg: s.backgroundColor, t: (e.innerText || '').trim().slice(0, 30) };
    });
  };
  for (const [k, v] of Object.entries(window.__boxes || {})) pick(k, v[0], v[1] || 1);
  return out;
})()
