/* Coverline: the ••• overflow menu, the mobile drawer and small touches. */
(function () {
  'use strict';
  var $ = function (s, r) { return (r || document).querySelector(s); };
  var $$ = function (s, r) { return Array.prototype.slice.call((r || document).querySelectorAll(s)); };

  /* nav links that do not fit before the centred title move into ••• */
  var list = $('[data-cv-nav]'), more = $('[data-cv-more]'), menu = $('[data-cv-more-menu]'), btn = $('[data-cv-more-btn]');
  if (list && more && menu) {
    var items = $$('.cv-nav-item', list);
    var fit = function () {
      items.forEach(function (li) { li.hidden = false; });
      menu.innerHTML = '';
      more.hidden = false;
      var room = list.parentElement.getBoundingClientRect().width - 44;
      var used = 0, overflow = [];
      items.forEach(function (li) { used += li.getBoundingClientRect().width + 22; if (used > room) overflow.push(li); });
      overflow.forEach(function (li) { var c = li.cloneNode(true); c.hidden = false; menu.appendChild(c); li.hidden = true; });
      more.hidden = overflow.length === 0;
    };
    if (document.fonts && document.fonts.ready) document.fonts.ready.then(fit); else fit();
    window.addEventListener('resize', fit);
    btn.addEventListener('click', function (e) { e.stopPropagation(); menu.hidden = !menu.hidden; btn.setAttribute('aria-expanded', String(!menu.hidden)); });
    document.addEventListener('click', function () { menu.hidden = true; btn.setAttribute('aria-expanded', 'false'); });
  }

  /* today's date, in the reader's own language */
  var date = $('[data-cv-date]');
  if (date) { try { date.textContent = new Date().toLocaleDateString(document.documentElement.lang || undefined, { weekday: 'long', day: 'numeric', month: 'long', year: 'numeric' }); } catch (e) {} }

  /* the slim bar slides in once the masthead has scrolled away */
  var bar = $('[data-cv-bar]'), head = $('#cv-head');
  if (bar && head) {
    var show = function () {
      var on = head.getBoundingClientRect().bottom < 0;
      bar.classList.toggle('is-on', on);
      bar.setAttribute('aria-hidden', String(!on));
      $$('a, button', bar).forEach(function (el) { el.tabIndex = on ? 0 : -1; });
    };
    window.addEventListener('scroll', show, { passive: true }); show();
  }

  /* mobile drawer */
  var drawer = $('[data-cv-drawer]'), burger = $('[data-cv-burger]');
  if (drawer && burger) {
    var set = function (on) { drawer.hidden = !on; burger.setAttribute('aria-expanded', String(on)); document.documentElement.style.overflow = on ? 'hidden' : ''; };
    burger.addEventListener('click', function () { set(true); });
    $('[data-cv-drawer-close]', drawer).addEventListener('click', function () { set(false); });
    document.addEventListener('keydown', function (e) { if (e.key === 'Escape') set(false); });
  }

  /* copy link */
  $$('[data-cv-copy]').forEach(function (b) {
    b.addEventListener('click', function () {
      if (!navigator.clipboard) return;
      navigator.clipboard.writeText(b.getAttribute('data-cv-copy')).then(function () { b.classList.add('is-done'); setTimeout(function () { b.classList.remove('is-done'); }, 1500); });
    });
  });

  /* toggle cards */
  $$('.kg-toggle-card').forEach(function (c) {
    var h = $('.kg-toggle-heading', c);
    if (h) h.addEventListener('click', function () { c.setAttribute('data-kg-toggle-state', c.getAttribute('data-kg-toggle-state') === 'close' ? 'open' : 'close'); });
  });
})();
