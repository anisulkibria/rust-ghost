/* Plinth: small, dependency-free behaviour. Everything works without it;
   this adds the slider, rail arrows, countdown, hotspots and menus. */
(function () {
  'use strict';
  var $$ = function (sel, root) { return Array.prototype.slice.call((root || document).querySelectorAll(sel)); };
  var reduce = window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches;

  /* swatches: material and colour names CSS does not know */
  var tones = { oak: '#C9A272', walnut: '#5E3E2A', ash: '#D8C7A6', natural: '#D9BF94', sand: '#D8C3A5', sage: '#9DAE93', olive: '#6F7442',
    tan: '#C08A5B', brass: '#B8954A', terracotta: '#C66A3D', ivory: '#F1EBDD', cream: '#EFE6D2', charcoal: '#3A3A38', navy: '#24305E',
    clay: '#C66A3D', linen: '#E7DFD0', smoke: '#9A9A96', white: '#F7F5F0', black: '#1C1B18', red: '#D8432B' };
  $$('.pl-swatch, .pl-pill-sw').forEach(function (el) {
    var name = (el.getAttribute('title') || el.parentElement.textContent || '').trim().toLowerCase();
    var key = Object.keys(tones).filter(function (k) { return name.indexOf(k) !== -1; })[0];
    if (key) el.style.backgroundColor = tones[key];
  });

  /* a drawn product takes the colour picked on its product page */
  var drawn = document.querySelector('.pl-gmain-drawn');
  $$('.pl-opt.is-color').forEach(function (set) {
    var paint = function () {
      var on = set.querySelector('input:checked');
      var sw = on && on.parentElement.querySelector('.pl-pill-sw');
      if (drawn && sw) drawn.style.setProperty('--pl-pick', getComputedStyle(sw).backgroundColor);
    };
    set.addEventListener('change', paint); paint();
  });

  /* reading progress on a story */
  var bar = document.querySelector('[data-pl-progress]'), story = document.querySelector('.pl-article .gh-content');
  if (bar && story) {
    var read = function () {
      var r = story.getBoundingClientRect(), total = r.height - window.innerHeight * 0.6;
      bar.style.transform = 'scaleX(' + Math.min(1, Math.max(0, -r.top / Math.max(1, total))) + ')';
    };
    window.addEventListener('scroll', read, { passive: true }); read();
  }

  /* copy a story's link */
  $$('[data-pl-copy]').forEach(function (b) {
    b.addEventListener('click', function () {
      if (!navigator.clipboard) return;
      navigator.clipboard.writeText(b.getAttribute('data-pl-copy')).then(function () {
        b.classList.add('is-done'); b.setAttribute('aria-label', 'Link copied');
        setTimeout(function () { b.classList.remove('is-done'); b.setAttribute('aria-label', 'Copy link'); }, 1800);
      });
    });
  });

  /* toggle cards in a story */
  $$('.kg-toggle-card').forEach(function (card) {
    var head = card.querySelector('.kg-toggle-heading');
    if (head) head.addEventListener('click', function () {
      card.setAttribute('data-kg-toggle-state', card.getAttribute('data-kg-toggle-state') === 'close' ? 'open' : 'close');
    });
  });

  /* sticky header shadow */
  var header = document.getElementById('pl-header');
  if (header) {
    var onScroll = function () { header.classList.toggle('is-stuck', window.scrollY > 8); };
    window.addEventListener('scroll', onScroll, { passive: true }); onScroll();
  }

  /* header search: the category select picks where the search goes */
  $$('[data-pl-search]').forEach(function (form) {
    var cat = form.querySelector('[data-pl-search-cat]');
    if (!cat) return;
    form.addEventListener('submit', function () { form.action = cat.value || '/shop/'; });
  });

  /* mega menu */
  $$('[data-pl-mega]').forEach(function (btn) {
    var panel = document.getElementById(btn.getAttribute('aria-controls'));
    if (!panel) return;
    var set = function (open) { btn.setAttribute('aria-expanded', String(open)); panel.hidden = !open; };
    btn.addEventListener('click', function (e) { e.stopPropagation(); set(panel.hidden); });
    document.addEventListener('click', function (e) { if (!panel.hidden && !panel.contains(e.target)) set(false); });
    document.addEventListener('keydown', function (e) { if (e.key === 'Escape' && !panel.hidden) { set(false); btn.focus(); } });
  });

  /* mobile menu */
  $$('[data-pl-menu]').forEach(function (btn) {
    var nav = document.getElementById(btn.getAttribute('aria-controls'));
    if (!nav) return;
    var close = document.createElement('button');
    close.type = 'button'; close.className = 'pl-round pl-menu-close'; close.hidden = true;
    close.setAttribute('aria-label', 'Close menu');
    close.innerHTML = '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M6 6l12 12M18 6 6 18"/></svg>';
    document.body.appendChild(close);
    var set = function (open) {
      nav.classList.toggle('is-open', open); close.hidden = !open;
      btn.setAttribute('aria-expanded', String(open));
      document.documentElement.style.overflow = open ? 'hidden' : '';
    };
    btn.addEventListener('click', function () { set(true); });
    close.addEventListener('click', function () { set(false); btn.focus(); });
    nav.addEventListener('click', function (event) {
      var link = event.target.closest('a[href^="#"]');
      if (link && nav.classList.contains('is-open')) set(false);
    });
    document.addEventListener('keydown', function (e) { if (e.key === 'Escape' && nav.classList.contains('is-open')) set(false); });
  });

  /* hero slider */
  $$('[data-pl-slider]').forEach(function (root) {
    var slides = $$('.pl-hero-slide', root), dots = $$('[data-pl-go]', root);
    if (slides.length < 2) { dots.forEach(function (d) { d.hidden = true; }); return; }
    var at = 0;
    var go = function (n) {
      at = (n + slides.length) % slides.length;
      slides.forEach(function (slide, i) {
        var active = i === at;
        slide.classList.toggle('is-on', active);
        slide.setAttribute('aria-hidden', String(!active));
        slide.toggleAttribute('inert', !active);
      });
      dots.forEach(function (dot, i) {
        dot.classList.toggle('is-on', i === at);
        dot.setAttribute('aria-pressed', String(i === at));
      });
    };
    dots.forEach(function (dot) { dot.addEventListener('click', function () { go(+dot.dataset.plGo); }); });
    var x0 = null;
    root.addEventListener('touchstart', function (e) { x0 = e.touches[0].clientX; }, { passive: true });
    root.addEventListener('touchend', function (e) {
      if (x0 === null) return;
      var dx = e.changedTouches[0].clientX - x0; x0 = null;
      if (Math.abs(dx) > 40) go(at + (dx < 0 ? 1 : -1));
    });
    go(0);
  });

  /* rails: arrows scroll the rail in the same section */
  $$('[data-pl-rail]').forEach(function (rail) {
    var section = rail.closest('section');
    if (!section) return;
    var prev = section.querySelector('[data-pl-prev]'), next = section.querySelector('[data-pl-next]');
    var step = function () { var c = rail.firstElementChild; return c ? c.getBoundingClientRect().width + 22 : rail.clientWidth; };
    var paint = function () {
      if (prev) prev.disabled = rail.scrollLeft <= 4;
      if (next) next.disabled = rail.scrollLeft + rail.clientWidth >= rail.scrollWidth - 4;
    };
    if (prev) prev.addEventListener('click', function () { rail.scrollBy({ left: -step(), behavior: 'smooth' }); });
    if (next) next.addEventListener('click', function () { rail.scrollBy({ left: step(), behavior: 'smooth' }); });
    rail.addEventListener('scroll', paint, { passive: true });
    window.addEventListener('resize', paint); paint();
  });

  /* drag to scroll the category strip with a mouse */

  /* flash sale countdown */
  $$('[data-pl-countdown]').forEach(function (band) {
    var raw = band.getAttribute('data-pl-countdown').trim();
    var end = new Date(/T/.test(raw) ? raw : raw + 'T23:59:59');
    if (isNaN(end)) { band.hidden = true; return; }
    var f = function (sel) { return band.querySelector(sel); };
    var pad = function (n) { return (n < 10 ? '0' : '') + n; };
    var tick = function () {
      var s = Math.max(0, Math.floor((end - new Date()) / 1000));
      if (s === 0) { band.hidden = true; return; }
      f('[data-d]').textContent = pad(Math.floor(s / 86400));
      f('[data-h]').textContent = pad(Math.floor(s % 86400 / 3600));
      f('[data-m]').textContent = pad(Math.floor(s % 3600 / 60));
      f('[data-s]').textContent = pad(s % 60);
      setTimeout(tick, 1000);
    };
    tick();
  });

  /* lookbook hotspots: one card open at a time */
  $$('.pl-look-scene').forEach(function (scene) {
    var dots = $$('.pl-spot-dot', scene);
    dots.forEach(function (dot) {
      dot.addEventListener('click', function () {
        dots.forEach(function (d) {
          var on = d === dot && d.getAttribute('aria-expanded') !== 'true';
          d.setAttribute('aria-expanded', String(on));
          document.getElementById(d.getAttribute('aria-controls')).hidden = !on;
        });
      });
    });
  });

  /* quantity stepper */
  $$('[data-pl-qty]').forEach(function (box) {
    var input = box.querySelector('input');
    $$('[data-step]', box).forEach(function (b) {
      b.addEventListener('click', function () {
        var max = +input.max || 999, v = (+input.value || 1) + +b.getAttribute('data-step');
        input.value = Math.min(max, Math.max(1, v));
      });
    });
  });

  /* sections rise in as they scroll into view */
  if (!reduce && 'IntersectionObserver' in window) {
    var io = new IntersectionObserver(function (entries) {
      entries.forEach(function (e) { if (e.isIntersecting) { e.target.classList.add('is-in'); io.unobserve(e.target); } });
    }, { rootMargin: '0px 0px -8% 0px' });
    $$('.pl-home > section:not(.pl-hero), .pl-grid > .pl-card').forEach(function (el, i) {
      el.classList.add('pl-reveal');
      if (el.classList.contains('pl-card')) el.style.transitionDelay = (i % 4) * 70 + 'ms';
      io.observe(el);
    });
  }
})();
