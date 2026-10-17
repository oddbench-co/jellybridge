// Progressive enhancement only: every page works without this file.
// 1. Download buttons point at the visitor's system. 2. The FAQ gets a filter box.
// 3. The screenshot carousel gets arrows, and thumbnails scroll it without jumping the page.
(() => {
  "use strict";

  const ua = navigator.userAgent;
  // userAgentData where the browser has it (Chromium), otherwise the user agent string.
  const platform = ((navigator.userAgentData && navigator.userAgentData.platform) || ua).toLowerCase();
  const os = /android|iphone|ipad|ipod|cros/i.test(ua) ? null
    : platform.includes("win") ? ["windows", "Windows"]
    : platform.includes("mac") ? ["macos", "macOS"]
    : platform.includes("linux") || platform.includes("x11") ? ["linux", "Linux"]
    : null;

  if (os) {
    for (const link of document.querySelectorAll("[data-os-download]")) {
      link.href = "/download#" + os[0];
      const label = link.querySelector("[data-os-label]");
      if (label) label.textContent = "Download for " + os[1];
    }
    const row = document.querySelector('[data-os="' + os[0] + '"]');
    if (row) {
      row.classList.add("is-yours");
      const tag = row.querySelector(".platform__yours");
      if (tag) tag.hidden = false;
    }
  }

  const filter = document.querySelector("[data-faq-filter]");
  if (filter) {
    const input = filter.querySelector("input");
    const items = Array.from(document.querySelectorAll(".faq details"));
    const empty = document.querySelector("[data-faq-empty]");
    const text = items.map((d) => d.textContent.toLowerCase());
    filter.hidden = false;
    input.addEventListener("input", () => {
      const words = input.value.toLowerCase().split(/\s+/).filter(Boolean);
      let shown = 0;
      items.forEach((d, i) => {
        const hit = words.every((w) => text[i].includes(w));
        d.hidden = !hit;
        if (hit) shown++;
      });
      empty.hidden = shown > 0;
    });
  }

  const carousel = document.querySelector("[data-carousel]");
  if (carousel) {
    const track = carousel.querySelector(".carousel__track");
    const slides = Array.from(track.children);
    const thumbs = Array.from(carousel.querySelectorAll("[data-carousel-thumb]"));
    const prev = carousel.querySelector("[data-carousel-prev]");
    const next = carousel.querySelector("[data-carousel-next]");
    const reduce = matchMedia("(prefers-reduced-motion: reduce)").matches;
    const offset = (n) => slides[n].offsetLeft - slides[0].offsetLeft;
    // While a smooth scroll runs, the slide it heads to counts as current, so quick repeated
    // presses advance one slide each instead of measuring a position halfway through.
    let target = -1;
    const current = () => (target >= 0 ? target : Math.round(track.scrollLeft / (offset(1) || 1)));
    const go = (i) => {
      const n = Math.max(0, Math.min(slides.length - 1, i));
      target = reduce ? -1 : n;
      track.scrollTo({ left: offset(n), behavior: reduce ? "auto" : "smooth" });
      if (reduce) sync();
    };
    const sync = () => {
      if (target >= 0 && Math.abs(track.scrollLeft - offset(target)) < 2) target = -1;
      const i = current();
      thumbs.forEach((t, n) => t.setAttribute("aria-current", n === i ? "true" : "false"));
      prev.disabled = i === 0;
      next.disabled = i === slides.length - 1;
    };
    prev.hidden = next.hidden = false;
    prev.addEventListener("click", () => go(current() - 1));
    next.addEventListener("click", () => go(current() + 1));
    thumbs.forEach((t, n) => t.addEventListener("click", (e) => { e.preventDefault(); go(n); }));
    track.addEventListener("keydown", (e) => {
      if (e.key === "ArrowLeft") { e.preventDefault(); go(current() - 1); }
      if (e.key === "ArrowRight") { e.preventDefault(); go(current() + 1); }
    });
    // A swipe or wheel takes over from a scroll the buttons started.
    for (const type of ["pointerdown", "wheel", "touchstart"]) track.addEventListener(type, () => { target = -1; }, { passive: true });
    let frame = 0;
    track.addEventListener("scroll", () => { cancelAnimationFrame(frame); frame = requestAnimationFrame(sync); }, { passive: true });
    sync();
  }
})();
