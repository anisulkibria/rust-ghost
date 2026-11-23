/*
 * Vuuut search — the built-in replacement for Sodo Search.
 *
 * It answers exactly what Sodo answers, so no theme has to change: any
 * element with `data-ghost-search`, any link to `#/search`, and a page
 * opened at `#/search`. With the shortcut on, "/" and ⌘K / Ctrl-K open it
 * too.
 *
 * Ranking, filtering and the members-only rule all happen on the server
 * (`/search/api/`); this script only draws what comes back. Every string is
 * placed with textContent; a highlight is a character range the server
 * computed, wrapped in <mark> here — the server never sends markup.
 *
 * A search is logged (if the site keeps insights) once, when the visitor
 * stops typing for a moment or picks a result — not on every keystroke.
 */
(function () {
  'use strict';

  var script = document.currentScript;
  if (!script) return;
  var cfg;
  try { cfg = JSON.parse(script.getAttribute('data-config') || '{}'); } catch (e) { return; }
  if (!cfg.endpoint) return;

  var root = null, input, list, chips, facetsRow, status;
  var filter = { type: '', tag: '' };
  var seq = 0, timer = null, logTimer = null, lastLogged = '', items = [], active = -1;
  var opener = null;

  function el(tag, cls, text) {
    var e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text != null) e.textContent = text;
    return e;
  }

  /** Text with server-computed [start, end) character ranges marked. */
  function marked(text, ranges) {
    var frag = document.createDocumentFragment();
    var chars = Array.from(text || '');
    var at = 0;
    (ranges || []).forEach(function (r) {
      if (r[0] < at || r[1] > chars.length) return;
      if (r[0] > at) frag.appendChild(document.createTextNode(chars.slice(at, r[0]).join('')));
      frag.appendChild(el('mark', null, chars.slice(r[0], r[1]).join('')));
      at = r[1];
    });
    if (at < chars.length) frag.appendChild(document.createTextNode(chars.slice(at).join('')));
    return frag;
  }

  function build() {
    root = el('div', 'vs-root');
    root.setAttribute('role', 'dialog');
    root.setAttribute('aria-modal', 'true');
    root.setAttribute('aria-label', 'Search');
    var panel = el('div', 'vs-panel');

    var bar = el('div', 'vs-bar');
    var icon = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
    icon.setAttribute('viewBox', '0 0 24 24');
    icon.setAttribute('class', 'vs-icon');
    icon.setAttribute('aria-hidden', 'true');
    var p = document.createElementNS('http://www.w3.org/2000/svg', 'path');
    p.setAttribute('d', 'M15.5 14h-.79l-.28-.27A6.47 6.47 0 0 0 16 9.5 6.5 6.5 0 1 0 9.5 16c1.61 0 3.09-.59 4.23-1.57l.27.28v.79l5 4.99L20.49 19l-4.99-5zm-6 0C7.01 14 5 11.99 5 9.5S7.01 5 9.5 5 14 7.01 14 9.5 11.99 14 9.5 14z');
    icon.appendChild(p);
    bar.appendChild(icon);
    input = el('input', 'vs-input');
    input.type = 'search';
    input.placeholder = cfg.placeholder || 'Search';
    input.setAttribute('aria-label', cfg.placeholder || 'Search');
    input.setAttribute('autocomplete', 'off');
    input.setAttribute('spellcheck', 'false');
    input.setAttribute('aria-controls', 'vs-list');
    bar.appendChild(input);
    var close = el('button', 'vs-close', 'Esc');
    close.type = 'button';
    close.setAttribute('aria-label', 'Close search');
    close.addEventListener('click', hide);
    bar.appendChild(close);
    panel.appendChild(bar);

    chips = el('div', 'vs-chips');
    chips.setAttribute('role', 'group');
    chips.setAttribute('aria-label', 'Filter by type');
    var types = [['', 'All'], ['post', 'Posts']];
    if (cfg.pages) types.push(['page', 'Pages']);
    types.forEach(function (t) {
      var c = el('button', 'vs-chip', t[1]);
      c.type = 'button';
      c.setAttribute('data-type', t[0]);
      c.setAttribute('aria-pressed', t[0] === filter.type ? 'true' : 'false');
      c.addEventListener('click', function () {
        filter.type = t[0];
        Array.prototype.forEach.call(chips.querySelectorAll('[data-type]'), function (x) {
          x.setAttribute('aria-pressed', x.getAttribute('data-type') === filter.type ? 'true' : 'false');
        });
        run(true);
        input.focus();
      });
      chips.appendChild(c);
    });
    facetsRow = el('div', 'vs-facets');
    chips.appendChild(facetsRow);
    panel.appendChild(chips);

    status = el('p', 'vs-status');
    status.setAttribute('role', 'status');
    status.setAttribute('aria-live', 'polite');
    list = el('div', 'vs-list');
    list.id = 'vs-list';
    var scroll = el('div', 'vs-scroll');
    scroll.appendChild(status);
    scroll.appendChild(list);
    panel.appendChild(scroll);

    var foot = el('div', 'vs-foot');
    foot.appendChild(el('span', null, '↑↓ to move · Enter to open · Esc to close'));
    panel.appendChild(foot);

    root.appendChild(panel);
    root.addEventListener('mousedown', function (e) { if (e.target === root) hide(); });
    input.addEventListener('input', function () {
      clearTimeout(timer);
      timer = setTimeout(function () { run(false); }, 140);
    });
    input.addEventListener('keydown', keys);
    root.addEventListener('keydown', function (e) {
      if (e.key === 'Escape') { e.preventDefault(); hide(); }
      if (e.key === 'Tab') trap(e);
    });
    document.body.appendChild(root);
    idle();
  }

  function trap(e) {
    var f = root.querySelectorAll('input, button, a[href]');
    if (!f.length) return;
    var first = f[0], last = f[f.length - 1];
    if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last.focus(); }
    else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first.focus(); }
  }

  function idle() {
    list.textContent = '';
    facetsRow.textContent = '';
    status.textContent = '';
    items = [];
    active = -1;
  }

  function keys(e) {
    if (e.key === 'ArrowDown') { e.preventDefault(); point(Math.min(active + 1, items.length - 1)); }
    else if (e.key === 'ArrowUp') { e.preventDefault(); point(Math.max(active - 1, 0)); }
    else if (e.key === 'Enter') {
      var target = items[active >= 0 ? active : 0];
      if (target) { e.preventDefault(); log(true); location.href = target.href; }
    }
  }

  function point(i) {
    items.forEach(function (a) { a.classList.remove('vs-on'); a.removeAttribute('aria-selected'); });
    active = i;
    var a = items[i];
    if (!a) return;
    a.classList.add('vs-on');
    a.setAttribute('aria-selected', 'true');
    a.scrollIntoView({ block: 'nearest' });
  }

  function url(logIt) {
    var u = cfg.endpoint + '?q=' + encodeURIComponent(input.value.trim());
    if (filter.type) u += '&type=' + encodeURIComponent(filter.type);
    if (filter.tag) u += '&tag=' + encodeURIComponent(filter.tag);
    if (logIt) u += '&log=1';
    return u;
  }

  /** Count this search once, when the visitor has settled on it. */
  function log(now) {
    clearTimeout(logTimer);
    if (!cfg.log) return;
    var q = input.value.trim();
    var send = function () {
      if (q.length < 2 || q === lastLogged) return;
      lastLogged = q;
      try { fetch(url(true), { credentials: 'omit', keepalive: true }).catch(function () {}); } catch (e) {}
    };
    if (now) send(); else logTimer = setTimeout(send, 1500);
  }

  function run(keepTag) {
    if (!keepTag) filter.tag = '';
    var q = input.value.trim();
    if (q.length < 1) { idle(); return; }
    var mine = ++seq;
    status.textContent = 'Searching…';
    fetch(url(false), { credentials: 'omit' })
      .then(function (r) { return r.ok ? r.json() : null; })
      .then(function (data) {
        if (mine !== seq || !data) return;
        draw(data, q);
        log(false);
      })
      .catch(function () { if (mine === seq) status.textContent = 'Search is not available right now.'; });
  }

  function section(title) {
    var h = el('p', 'vs-section', title);
    list.appendChild(h);
  }

  function link(href, cls) {
    var a = el('a', 'vs-item ' + (cls || ''));
    a.href = href;
    a.setAttribute('role', 'option');
    a.addEventListener('click', function () { log(true); });
    a.addEventListener('mousemove', function () { point(items.indexOf(a)); });
    items.push(a);
    return a;
  }

  function draw(data, q) {
    list.textContent = '';
    items = [];
    active = -1;

    facetsRow.textContent = '';
    (data.facets || []).forEach(function (f) {
      var c = el('button', 'vs-chip vs-tag', '# ' + f.name);
      c.type = 'button';
      c.setAttribute('aria-pressed', filter.tag === f.slug ? 'true' : 'false');
      c.addEventListener('click', function () {
        filter.tag = filter.tag === f.slug ? '' : f.slug;
        run(true);
        input.focus();
      });
      facetsRow.appendChild(c);
    });

    var people = data.authors || [], tags = data.tags || [], res = data.results || [];
    if (tags.length) {
      section('Tags');
      var row = el('div', 'vs-pills');
      tags.forEach(function (t) {
        var a = link(t.url, 'vs-pill');
        a.textContent = '# ' + t.name;
        row.appendChild(a);
      });
      list.appendChild(row);
    }
    if (people.length) {
      section('Authors');
      var prow = el('div', 'vs-pills');
      people.forEach(function (t) {
        var a = link(t.url, 'vs-pill');
        a.textContent = t.name;
        prow.appendChild(a);
      });
      list.appendChild(prow);
    }
    if (res.length) {
      section(filter.type === 'page' ? 'Pages' : (filter.type === 'post' ? 'Posts' : 'Posts and pages'));
      res.forEach(function (r) {
        var a = link(r.url, 'vs-result');
        var text = el('span', 'vs-text');
        var t = el('span', 'vs-title');
        t.appendChild(marked(r.title, r.titleMarks));
        text.appendChild(t);
        if (r.excerpt) {
          var x = el('span', 'vs-excerpt');
          x.appendChild(marked(r.excerpt, r.marks));
          text.appendChild(x);
        }
        var meta = el('span', 'vs-meta');
        var bits = [];
        if (r.kind === 'page') bits.push('Page');
        if (r.date) bits.push(r.date);
        if (r.tags && r.tags.length) bits.push(r.tags.join(', '));
        meta.textContent = bits.join(' · ');
        if (r.members) meta.appendChild(el('span', 'vs-badge', 'Members'));
        text.appendChild(meta);
        a.appendChild(text);
        if (r.image) {
          var img = el('img', 'vs-thumb');
          img.src = r.image;
          img.alt = '';
          img.loading = 'lazy';
          a.appendChild(img);
        }
        list.appendChild(a);
      });
    }
    var total = data.total || 0;
    if (!res.length && !tags.length && !people.length) {
      status.textContent = '';
      var empty = el('div', 'vs-empty');
      empty.appendChild(el('p', 'vs-empty-title', 'No results for “' + q + '”'));
      empty.appendChild(el('p', null, filter.tag || filter.type ? 'Try removing a filter, or a different word.' : 'Check the spelling, or try a broader word.'));
      list.appendChild(empty);
    } else {
      status.textContent = total === 1 ? '1 result' : total + ' results';
    }
    if (items.length) point(0);
  }

  function show(e) {
    if (e) e.preventDefault();
    if (!root) build();
    opener = document.activeElement;
    root.classList.add('vs-open');
    document.documentElement.classList.add('vs-lock');
    input.focus();
    input.select();
  }

  function hide() {
    if (!root || !root.classList.contains('vs-open')) return;
    log(true);
    root.classList.remove('vs-open');
    document.documentElement.classList.remove('vs-lock');
    if (location.hash === '#/search') history.replaceState(null, '', location.pathname + location.search);
    if (opener && opener.focus) opener.focus();
  }

  document.addEventListener('click', function (e) {
    var t = e.target && e.target.closest && e.target.closest('[data-ghost-search], a[href="#/search"], a[href$="/#/search"]');
    if (t) show(e);
  }, true);
  document.addEventListener('keydown', function (e) {
    if (!cfg.shortcut) return;
    var typing = /^(input|textarea|select)$/i.test(e.target.tagName || '') || e.target.isContentEditable;
    if ((e.key === 'k' || e.key === 'K') && (e.metaKey || e.ctrlKey)) { show(e); return; }
    if (e.key === '/' && !typing && !e.metaKey && !e.ctrlKey && !e.altKey) show(e);
  });
  window.addEventListener('hashchange', function () { if (location.hash === '#/search') show(); });
  if (location.hash === '#/search') {
    if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', function () { show(); });
    else show();
  }
  window.vuuutSearch = { open: show, close: hide };
})();
