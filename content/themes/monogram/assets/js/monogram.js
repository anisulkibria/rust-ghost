/* Monogram: behaviour only; the page works without it. */
(function () {
  'use strict';
  var $$ = function (s, r) { return Array.prototype.slice.call((r || document).querySelectorAll(s)); };
  var reduce = window.matchMedia('(prefers-reduced-motion: reduce)').matches;

  /* menu */
  var burger = document.querySelector('[data-mg-menu]'), nav = document.getElementById('mg-nav');
  if (burger && nav) {
    var set = function (open) { nav.classList.toggle('is-open', open); burger.setAttribute('aria-expanded', String(open)); };
    burger.addEventListener('click', function () { set(!nav.classList.contains('is-open')); });
    $$('a', nav).forEach(function (a) { a.addEventListener('click', function () { set(false); }); });
    document.addEventListener('keydown', function (e) { if (e.key === 'Escape') set(false); });
  }

  /* on the one-page home, the nav marks the section in view */
  var links = $$('.mg-nav-link[href^="#"]');
  if (links.length && 'IntersectionObserver' in window) {
    var spy = new IntersectionObserver(function (entries) {
      entries.forEach(function (e) {
        if (!e.isIntersecting) return;
        links.forEach(function (l) { l.classList.toggle('is-on', l.getAttribute('href') === '#' + e.target.id); });
      });
    }, { rootMargin: '-45% 0px -50% 0px' });
    links.forEach(function (l) { var t = document.querySelector(l.getAttribute('href')); if (t) spy.observe(t); });
  }

  /* rotating roles, typed */
  $$('[data-mg-roles]').forEach(function (el) {
    var roles = el.getAttribute('data-mg-roles').split('|').map(function (r) { return r.trim(); }).filter(Boolean);
    if (roles.length < 2 || reduce) return;
    var i = 0, text = roles[0], deleting = true;
    var tick = function () {
      if (deleting) {
        text = text.slice(0, -1);
        if (!text) { deleting = false; i = (i + 1) % roles.length; }
        el.textContent = text; setTimeout(tick, 45);
      } else {
        text = roles[i].slice(0, text.length + 1); el.textContent = text;
        if (text === roles[i]) { deleting = true; setTimeout(tick, 2200); } else setTimeout(tick, 85);
      }
    };
    setTimeout(tick, 2400);
  });

  /* project filter: one button per public tag the projects carry */
  $$('[data-mg-filter]').forEach(function (bar) {
    var grid = document.querySelector('[data-mg-filtered]');
    if (!grid) return;
    var cards = $$('.mg-project', grid), seen = {};
    cards.forEach(function (c) { (c.getAttribute('data-tags') || '').split('|').filter(Boolean).forEach(function (t) { seen[t] = true; }); });
    var names = Object.keys(seen);
    if (names.length < 2) { bar.hidden = true; return; }
    names.forEach(function (n) {
      var b = document.createElement('button'); b.type = 'button'; b.className = 'mg-filter-btn'; b.textContent = n; b.setAttribute('data-f', n); bar.appendChild(b);
    });
    bar.addEventListener('click', function (e) {
      var b = e.target.closest('.mg-filter-btn'); if (!b) return;
      var f = b.getAttribute('data-f');
      $$('.mg-filter-btn', bar).forEach(function (x) { x.classList.toggle('is-on', x === b); x.setAttribute('aria-pressed', String(x === b)); });
      cards.forEach(function (c) { c.classList.toggle('is-out', f !== '*' && (c.getAttribute('data-tags') || '').split('|').indexOf(f) === -1); });
    });
  });

  /* résumé tabs */
  var tabs = $$('.mg-tab');
  tabs.forEach(function (t) {
    t.addEventListener('click', function () {
      tabs.forEach(function (x) {
        var on = x === t; x.classList.toggle('is-on', on); x.setAttribute('aria-selected', String(on));
        var p = document.getElementById(x.getAttribute('aria-controls')); if (p) p.hidden = !on;
      });
      var skills = document.querySelector('.mg-skills');
      if (skills && t.getAttribute('aria-controls') === 'mg-tab-skl') requestAnimationFrame(function () { skills.classList.add('is-in'); });
    });
  });

  /* testimonials */
  $$('[data-mg-slider]').forEach(function (box) {
    var items = $$('.mg-quote', box), at = 0, timer;
    if (items.length < 2) { $$('.mg-slider-btns', box).forEach(function (b) { b.hidden = true; }); return; }
    var go = function (n) {
      at = (n + items.length) % items.length;
      items.forEach(function (q, i) { q.classList.toggle('is-on', i === at); });
      clearTimeout(timer); if (!reduce) timer = setTimeout(function () { go(at + 1); }, 8000);
    };
    box.querySelector('[data-mg-prev]').addEventListener('click', function () { go(at - 1); });
    box.querySelector('[data-mg-next]').addEventListener('click', function () { go(at + 1); });
    go(0);
  });

  /* contact form → the visitor's email app, message filled in */
  $$('[data-mg-mail]').forEach(function (form) {
    form.addEventListener('submit', function (e) {
      e.preventDefault();
      var v = function (n) { return (form.elements[n] && form.elements[n].value || '').trim(); };
      var body = v('message') + '\n\n— ' + v('name') + ' <' + v('email') + '>';
      window.location.href = 'mailto:' + form.getAttribute('data-mg-mail') + '?subject=' + encodeURIComponent(v('subject')) + '&body=' + encodeURIComponent(body);
    });
  });

  /* reading progress */
  var bar = document.querySelector('[data-mg-progress]'), story = document.querySelector('.mg-prose');
  if (bar && story) {
    var read = function () { var r = story.getBoundingClientRect(); bar.style.transform = 'scaleX(' + Math.min(1, Math.max(0, -r.top / Math.max(1, r.height - innerHeight * .5))) + ')'; };
    window.addEventListener('scroll', read, { passive: true }); read();
  }

  /* toggle cards */
  $$('.kg-toggle-card').forEach(function (c) {
    var h = c.querySelector('.kg-toggle-heading');
    if (h) h.addEventListener('click', function () { c.setAttribute('data-kg-toggle-state', c.getAttribute('data-kg-toggle-state') === 'close' ? 'open' : 'close'); });
  });

  /* reveal, and the project counter */
  if ('IntersectionObserver' in window) {
    var io = new IntersectionObserver(function (entries) {
      entries.forEach(function (e) {
        if (!e.isIntersecting) return;
        e.target.classList.add('is-in'); io.unobserve(e.target);
      });
    }, { rootMargin: '0px 0px -8% 0px' });
    $$('.mg-svc, .mg-project, .mg-post, .mg-job, .mg-head, .mg-quote-card, .mg-contact > *').forEach(function (el, i) {
      if (!reduce) { el.classList.add('mg-reveal'); el.style.transitionDelay = (i % 3) * 80 + 'ms'; }
      io.observe(el);
    });
  }
  $$('[data-mg-count]').forEach(function (el) {
    var end = parseInt(el.textContent, 10); if (!end || reduce) return;
    var t0 = null; var step = function (t) { if (!t0) t0 = t; var p = Math.min(1, (t - t0) / 1600); el.textContent = Math.round(end * (1 - Math.pow(1 - p, 3))); if (p < 1) requestAnimationFrame(step); };
    requestAnimationFrame(step);
  });
})();
