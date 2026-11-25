/* Outrider: behaviour only; every page reads fine without it. */
(function () {
  'use strict';
  var $$ = function (s, r) { return Array.prototype.slice.call((r || document).querySelectorAll(s)); };
  var reduce = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  var root = document.documentElement;

  /* header: solid once scrolled, hides going down, returns going up */
  var header = document.getElementById('or-header'), lastY = 0;
  /* display type widens as the page scrolls: 100% at the top, 140% by one screen */
  var stretchers = $$('[data-or-stretch]');
  var onScroll = function () {
    var y = window.scrollY;
    if (header) {
      header.classList.toggle('is-scrolled', y > 20);
      header.classList.toggle('is-hidden', y > 400 && y > lastY && !document.querySelector('.or-nav.is-open'));
    }
    lastY = y;
    if (!reduce) {
      stretchers.forEach(function (el) {
        var r = el.getBoundingClientRect();
        var t = 1 - Math.min(1, Math.max(0, (r.top + r.height / 2) / window.innerHeight));
        el.style.setProperty('--stretch', (78 + t * 52).toFixed(1));
      });
      var par = document.querySelector('[data-or-parallax] img');
      if (par) {
        var pr = par.parentElement.getBoundingClientRect();
        par.style.setProperty('--py', (-(pr.top / window.innerHeight) * 9).toFixed(2) + '%');
      }
    }
    var bar = document.querySelector('[data-or-progress]'), story = document.querySelector('.or-prose');
    if (bar && story) {
      var sr = story.getBoundingClientRect();
      bar.style.transform = 'scaleX(' + Math.min(1, Math.max(0, -sr.top / Math.max(1, sr.height - window.innerHeight * .5))) + ')';
    }
  };
  window.addEventListener('scroll', onScroll, { passive: true }); onScroll();

  /* mobile menu */
  var menu = document.querySelector('[data-or-menu]'), nav = document.getElementById('or-nav');
  if (menu && nav) {
    var set = function (open) { nav.classList.toggle('is-open', open); menu.setAttribute('aria-expanded', String(open)); root.style.overflow = open ? 'hidden' : ''; };
    menu.addEventListener('click', function () { set(!nav.classList.contains('is-open')); });
    document.addEventListener('keydown', function (e) { if (e.key === 'Escape') set(false); });
    $$('a', nav).forEach(function (a) { a.addEventListener('click', function () { set(false); }); });
  }

  /* cursor bubble over case studies */
  var cursor = document.querySelector('.or-cursor');
  if (cursor && window.matchMedia('(hover: hover)').matches) {
    window.addEventListener('pointermove', function (e) {
      cursor.style.setProperty('--x', e.clientX + 'px'); cursor.style.setProperty('--y', e.clientY + 'px');
    }, { passive: true });
    $$('[data-or-view]').forEach(function (el) {
      el.addEventListener('pointerenter', function () { cursor.classList.add('is-on'); });
      el.addEventListener('pointerleave', function () { cursor.classList.remove('is-on'); });
    });
  }

  /* magnetic buttons */
  if (!reduce) $$('[data-or-magnet]').forEach(function (el) {
    el.addEventListener('pointermove', function (e) {
      var r = el.getBoundingClientRect();
      el.style.transform = 'translate(' + ((e.clientX - r.left - r.width / 2) * .18) + 'px,' + ((e.clientY - r.top - r.height / 2) * .3) + 'px)';
    });
    el.addEventListener('pointerleave', function () { el.style.transform = ''; });
  });

  /* the studio statement lights up word by word as it scrolls past */
  $$('[data-or-words]').forEach(function (el) {
    if (reduce) return;
    var words = el.textContent.trim().split(/\s+/);
    el.innerHTML = words.map(function (w) { var s = document.createElement('span'); s.className = 'or-word'; s.textContent = w; return s.outerHTML; }).join(' ');
    var spans = $$('.or-word', el);
    var light = function () {
      var r = el.getBoundingClientRect();
      var t = Math.min(1, Math.max(0, (window.innerHeight * .85 - r.top) / (r.height + window.innerHeight * .35)));
      var n = Math.round(t * spans.length);
      spans.forEach(function (s, i) { s.classList.toggle('is-lit', i < n); });
    };
    window.addEventListener('scroll', light, { passive: true }); light();
  });

  /* count the stats up once */
  var count = function (el) {
    var raw = el.textContent.trim(), m = raw.match(/^(\D*)(\d+)(.*)$/);
    if (!m || reduce) return;
    var end = +m[2], t0 = null;
    var step = function (t) {
      if (!t0) t0 = t;
      var p = Math.min(1, (t - t0) / 1400), e = 1 - Math.pow(1 - p, 4);
      el.textContent = m[1] + Math.round(end * e) + m[3];
      if (p < 1) requestAnimationFrame(step);
    };
    requestAnimationFrame(step);
  };

  /* reveal on scroll */
  if ('IntersectionObserver' in window) {
    var io = new IntersectionObserver(function (entries) {
      entries.forEach(function (e) {
        if (!e.isIntersecting) return;
        e.target.classList.add('is-in');
        if (e.target.hasAttribute('data-or-count')) count(e.target);
        io.unobserve(e.target);
      });
    }, { rootMargin: '0px 0px -10% 0px' });
    $$('.or-work, .or-post, .or-person, .or-step, .or-head, .or-stats, .or-svc').forEach(function (el, i) {
      if (!reduce) el.classList.add('or-reveal');
      io.observe(el);
    });
    $$('[data-or-count]').forEach(function (el) { io.observe(el); });
  }

  /* testimonials */
  $$('[data-or-slider]').forEach(function (box) {
    var items = $$('.or-voice', box), at = 0, timer;
    var cur = box.querySelector('[data-or-current]'), tot = box.querySelector('[data-or-total]');
    var pad = function (n) { return (n < 10 ? '0' : '') + n; };
    if (tot) tot.textContent = pad(items.length);
    if (items.length < 2) { $$('.or-slider-btns', box).forEach(function (b) { b.hidden = true; }); return; }
    var go = function (n) {
      at = (n + items.length) % items.length;
      items.forEach(function (it, i) { it.classList.toggle('is-on', i === at); });
      if (cur) cur.textContent = pad(at + 1);
      clearTimeout(timer); if (!reduce) timer = setTimeout(function () { go(at + 1); }, 7000);
    };
    box.querySelector('[data-or-prev]').addEventListener('click', function () { go(at - 1); });
    box.querySelector('[data-or-next]').addEventListener('click', function () { go(at + 1); });
    go(0);
  });

  /* copy link */
  $$('[data-or-copy]').forEach(function (b) {
    b.addEventListener('click', function () {
      if (!navigator.clipboard) return;
      navigator.clipboard.writeText(b.getAttribute('data-or-copy')).then(function () {
        var was = b.textContent; b.textContent = 'Copied ✓'; setTimeout(function () { b.textContent = was; }, 1600);
      });
    });
  });

  /* toggle cards */
  $$('.kg-toggle-card').forEach(function (c) {
    var h = c.querySelector('.kg-toggle-heading');
    if (h) h.addEventListener('click', function () { c.setAttribute('data-kg-toggle-state', c.getAttribute('data-kg-toggle-state') === 'close' ? 'open' : 'close'); });
  });
})();
