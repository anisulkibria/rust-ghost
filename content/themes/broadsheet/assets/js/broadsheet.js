/* Broadsheet: behaviour only; every page reads fine without it. */
(function () {
  'use strict';
  var $ = function (s, r) { return (r || document).querySelector(s); };
  var $$ = function (s, r) { return Array.prototype.slice.call((r || document).querySelectorAll(s)); };
  var root = document.documentElement;
  var store = {
    get: function (k, d) { try { return JSON.parse(localStorage.getItem(k)) || d; } catch (e) { return d; } },
    set: function (k, v) { try { localStorage.setItem(k, JSON.stringify(v)); } catch (e) {} }
  };

  /* clock + today's date in the reader's own time */
  var clock = $('[data-np-clock]');
  if (clock) {
    var zone = clock.getAttribute('data-zone') || '';
    var tick = function () {
      var d = new Date();
      clock.textContent = ('0' + d.getHours()).slice(-2) + ':' + ('0' + d.getMinutes()).slice(-2) + (zone ? ' ' + zone : '');
    };
    tick(); setInterval(tick, 15000);
  }
  var date = $('[data-np-date]');
  if (date) {
    try { date.textContent = new Date().toLocaleDateString(root.lang || undefined, { weekday: 'long', day: 'numeric', month: 'long', year: 'numeric' }); } catch (e) {}
  }

  /* ticker: pause and pin */
  var ticker = $('.np-ticker');
  if (ticker) {
    var pause = $('[data-np-ticker-pause]', ticker), pin = $('[data-np-ticker-pin]', ticker);
    if (pause) pause.addEventListener('click', function () { var on = ticker.classList.toggle('is-paused'); pause.setAttribute('aria-pressed', String(on)); });
    if (pin) {
      var pinned = store.get('np-pin', false);
      var setPin = function (on) { ticker.classList.toggle('is-pinned', on); pin.setAttribute('aria-pressed', String(on)); store.set('np-pin', on); };
      setPin(pinned);
      pin.addEventListener('click', function () { setPin(!ticker.classList.contains('is-pinned')); });
    }
  }

  /* dark mode */
  $$('[data-np-theme]').forEach(function (b) {
    if (b === root) return;
    b.addEventListener('click', function () {
      var next = root.getAttribute('data-np-theme') === 'dark' ? 'light' : 'dark';
      root.setAttribute('data-np-theme', next);
      try { localStorage.setItem('np-theme', next); } catch (e) {}
    });
  });

  /* sections drawer */
  var drawer = document.getElementById('np-drawer'), drawerBtn = $('[data-np-drawer]');
  if (drawer && drawerBtn) {
    var openDrawer = function (on) { drawer.hidden = !on; drawerBtn.setAttribute('aria-expanded', String(on)); root.style.overflow = on ? 'hidden' : ''; };
    drawerBtn.addEventListener('click', function () { openDrawer(true); });
    drawer.addEventListener('click', function (e) { if (e.target === drawer || e.target.closest('[data-np-drawer-close]')) openDrawer(false); });
    document.addEventListener('keydown', function (e) { if (e.key === 'Escape') openDrawer(false); });
  }

  /* saved stories, kept in this browser */
  var saved = store.get('np-saved', []);
  var panel = document.getElementById('np-saved-panel'), savedBtn = $('[data-np-saved]');
  var paintSaved = function () {
    var count = savedBtn && $('.np-saved-count', savedBtn);
    if (count) { count.hidden = !saved.length; count.textContent = saved.length; }
    $$('[data-np-save]').forEach(function (b) {
      var on = saved.some(function (s) { return s.url === b.getAttribute('data-url'); });
      b.classList.toggle('is-on', on); var label = $('span', b); if (label) label.textContent = on ? 'Saved' : 'Save';
    });
    if (panel) {
      var list = $('[data-np-saved-list]', panel);
      list.innerHTML = '';
      saved.forEach(function (s) { var li = document.createElement('li'); var a = document.createElement('a'); a.href = s.url; a.textContent = s.title; li.appendChild(a); list.appendChild(li); });
      $('[data-np-saved-empty]', panel).hidden = !!saved.length;
    }
  };
  $$('[data-np-save]').forEach(function (b) {
    b.addEventListener('click', function () {
      var url = b.getAttribute('data-url');
      var i = saved.findIndex(function (s) { return s.url === url; });
      if (i > -1) saved.splice(i, 1); else saved.unshift({ url: url, title: b.getAttribute('data-title') });
      store.set('np-saved', saved.slice(0, 50)); paintSaved();
    });
  });
  if (savedBtn && panel) {
    savedBtn.addEventListener('click', function (e) { e.stopPropagation(); panel.hidden = !panel.hidden; });
    document.addEventListener('click', function (e) { if (!panel.hidden && !panel.contains(e.target)) panel.hidden = true; });
  }
  paintSaved();

  /* rails: the arrows in a section scroll its rail */
  $$('[data-np-rail-wrap]').forEach(function (wrap) {
    var rail = $('[data-np-rail]', wrap), prev = $('[data-np-prev]', wrap), next = $('[data-np-next]', wrap);
    if (!rail) return;
    var step = function () { var c = rail.firstElementChild; return c ? c.getBoundingClientRect().width + 24 : rail.clientWidth; };
    var paint = function () {
      if (prev) prev.disabled = rail.scrollLeft < 4;
      if (next) next.disabled = rail.scrollLeft + rail.clientWidth >= rail.scrollWidth - 4;
    };
    if (prev) prev.addEventListener('click', function () { rail.scrollBy({ left: -step(), behavior: 'smooth' }); });
    if (next) next.addEventListener('click', function () { rail.scrollBy({ left: step(), behavior: 'smooth' }); });
    rail.addEventListener('scroll', paint, { passive: true }); window.addEventListener('resize', paint); paint();
  });

  /* membership: monthly / yearly prices, and the chosen plan */
  $$('[data-np-plans]').forEach(function (box) {
    $$('[data-np-cycle]', box).forEach(function (b) {
      b.addEventListener('click', function () {
        var c = b.getAttribute('data-np-cycle');
        $$('[data-np-cycle]', box).forEach(function (x) { x.classList.toggle('is-on', x === b); });
        $$('[data-cycle]', box).forEach(function (p) { p.hidden = p.getAttribute('data-cycle') !== c; });
      });
    });
    var paid = $('input[data-type="paid"]', box) || $('input[name="np-plan"]', box);
    if (paid) paid.checked = true;
  });

  /* the gold words in the membership title are written *like this* */
  $$('[data-np-gold]').forEach(function (h) {
    var t = h.textContent, out = document.createDocumentFragment(), parts = t.split('*');
    parts.forEach(function (p, i) { if (!p) return; if (i % 2) { var em = document.createElement('em'); em.textContent = p; out.appendChild(em); } else out.appendChild(document.createTextNode(p)); });
    h.textContent = ''; h.appendChild(out);
  });

  /* footer: "#News" headings lose the # */
  $$('.np-footer-h').forEach(function (h) { h.textContent = h.textContent.replace(/^#\s*/, ''); });

  /* "Showing 1–18 of 287 stories" */
  $$('[data-np-showing]').forEach(function (p) {
    var page = +p.getAttribute('data-page'), limit = +p.getAttribute('data-limit'), total = +p.getAttribute('data-total');
    if (!page || !limit || !total) return;
    p.textContent = 'Showing ' + ((page - 1) * limit + 1) + '–' + Math.min(total, page * limit) + ' of ' + total + ' stories';
  });

  /* reading progress */
  var bar = $('[data-np-progress]'), story = $('.np-prose');
  if (bar && story) {
    var read = function () { var r = story.getBoundingClientRect(); bar.style.transform = 'scaleX(' + Math.min(1, Math.max(0, -r.top / Math.max(1, r.height - innerHeight * .5))) + ')'; };
    window.addEventListener('scroll', read, { passive: true }); read();
  }

  /* toggle cards */
  $$('.kg-toggle-card').forEach(function (c) {
    var h = $('.kg-toggle-heading', c);
    if (h) h.addEventListener('click', function () { c.setAttribute('data-kg-toggle-state', c.getAttribute('data-kg-toggle-state') === 'close' ? 'open' : 'close'); });
  });
})();
