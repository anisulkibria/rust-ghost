/* Wayletter: the overflow menu, Load more and small touches.
   Everything works without it; links fall back to plain pages. */
(function () {
  'use strict';
  var $ = function (s, r) { return (r || document).querySelector(s); };
  var $$ = function (s, r) { return Array.prototype.slice.call((r || document).querySelectorAll(s)); };

  /* nav: links that do not fit move into the ••• menu */
  var list = $('[data-ms-nav]'), more = $('[data-ms-more]'), menu = $('[data-ms-more-menu]'), moreBtn = $('[data-ms-more-btn]');
  if (list && more && menu) {
    var items = $$('.ms-nav-item', list);
    var fit = function () {
      items.forEach(function (li) { list.appendChild(li); li.hidden = false; });
      menu.innerHTML = '';
      more.hidden = false;
      var room = list.parentElement.getBoundingClientRect().width - more.getBoundingClientRect().width - 26;
      var overflow = [];
      var used = 0;
      items.forEach(function (li) {
        used += li.getBoundingClientRect().width + 23;
        if (used > room) overflow.push(li);
      });
      overflow.forEach(function (li) { var c = li.cloneNode(true); c.hidden = false; menu.appendChild(c); li.hidden = true; });
      more.hidden = overflow.length === 0;
    };
    if (document.fonts && document.fonts.ready) document.fonts.ready.then(fit); else fit();
    window.addEventListener('resize', fit);
    moreBtn.addEventListener('click', function (e) {
      e.stopPropagation();
      menu.hidden = !menu.hidden; moreBtn.setAttribute('aria-expanded', String(!menu.hidden));
    });
    document.addEventListener('click', function () { menu.hidden = true; moreBtn.setAttribute('aria-expanded', 'false'); });
  }

  /* mobile drawer */
  var drawer = $('[data-ms-drawer]'), burger = $('[data-ms-burger]');
  if (drawer && burger) {
    var set = function (on) { drawer.hidden = !on; burger.setAttribute('aria-expanded', String(on)); document.documentElement.style.overflow = on ? 'hidden' : ''; };
    burger.addEventListener('click', function () { set(true); });
    $('[data-ms-drawer-close]', drawer).addEventListener('click', function () { set(false); });
    document.addEventListener('keydown', function (e) { if (e.key === 'Escape') set(false); });
  }

  /* Load more: fetch the next page and append its rows */
  $$('[data-ms-load]').forEach(function (btn) {
    btn.addEventListener('click', function (e) {
      if (!window.fetch || !window.DOMParser) return;
      e.preventDefault();
      var feed = $('[data-ms-feed]');
      btn.classList.add('is-loading');
      fetch(btn.href).then(function (r) { return r.text(); }).then(function (html) {
        var doc = new DOMParser().parseFromString(html, 'text/html');
        $$('[data-ms-feed] .ms-feed-item', doc).forEach(function (item) { item.classList.add('is-new'); feed.appendChild(item); });
        var nextLink = $('[data-ms-load]', doc);
        if (nextLink) { btn.href = nextLink.getAttribute('href'); btn.classList.remove('is-loading'); }
        else btn.parentElement.remove();
      }).catch(function () { window.location.href = btn.href; });
    });
  });

  /* words written *like this* in a setting are set in the italic accent */
  $$('[data-ms-accent]').forEach(function (el) {
    var parts = el.textContent.split('*');
    if (parts.length < 3) return;
    el.textContent = '';
    parts.forEach(function (p, i) {
      if (!p) return;
      if (i % 2) { var em = document.createElement('em'); em.textContent = p; el.appendChild(em); }
      else el.appendChild(document.createTextNode(p));
    });
  });

  /* header gets a hairline once the page scrolls */
  var head = $('#ms-head');
  if (head) {
    var stick = function () { head.classList.toggle('is-stuck', window.scrollY > 8); };
    window.addEventListener('scroll', stick, { passive: true }); stick();
  }

  /* sections fade up as they arrive */
  if ('IntersectionObserver' in window && !window.matchMedia('(prefers-reduced-motion: reduce)').matches) {
    var io = new IntersectionObserver(function (entries) {
      entries.forEach(function (e) { if (e.isIntersecting) { e.target.classList.add('is-in'); io.unobserve(e.target); } });
    }, { rootMargin: '0px 0px -8% 0px' });
    $$('.ms-lead, .ms-card, .ms-feed-head, .ms-band-in').forEach(function (el, i) {
      el.classList.add('ms-reveal'); el.style.transitionDelay = (i % 3) * 70 + 'ms'; io.observe(el);
    });
  }

  /* copy link */
  $$('[data-ms-copy]').forEach(function (b) {
    b.addEventListener('click', function () {
      if (!navigator.clipboard) return;
      navigator.clipboard.writeText(b.getAttribute('data-ms-copy')).then(function () {
        b.classList.add('is-done'); setTimeout(function () { b.classList.remove('is-done'); }, 1500);
      });
    });
  });

  /* toggle cards */
  $$('.kg-toggle-card').forEach(function (c) {
    var h = $('.kg-toggle-heading', c);
    if (h) h.addEventListener('click', function () { c.setAttribute('data-kg-toggle-state', c.getAttribute('data-kg-toggle-state') === 'close' ? 'open' : 'close'); });
  });
})();
