/*
 * The Tools hub's search box.
 *
 * Every card carries `data-find`: escaped JSON of the phrases on that tool's
 * own screens (headings, fields) and the names of what staff made in it
 * (popups, forms, redirects…). The server builds that list —
 * `crate::tool_finder` — and decides which cards this viewer may see at all;
 * this script only hides the others and says what matched. Matches are
 * placed with textContent.
 *
 * Every word typed has to match somewhere on the card (title, description or
 * a phrase), accents and case ignored. Cards whose title matches come first;
 * the rest keep the A–Z order. The query lives in `?q=` so Back returns to
 * the same results, and "/" is left to the sidebar's own search.
 */
(function () {
  'use strict';
  var input = document.getElementById('tool-find');
  var grid = document.getElementById('tool-grid');
  if (!input || !grid) return;
  var none = document.getElementById('tool-none');
  var noneQ = document.getElementById('tool-none-q');
  var count = document.getElementById('tool-find-count');

  function fold(s) {
    return (s || '').toLowerCase().normalize('NFD').replace(/[\u0300-\u036f]/g, '');
  }

  var cards = Array.prototype.slice.call(grid.querySelectorAll('.tool-card')).map(function (el, i) {
    var phrases = [];
    try { phrases = JSON.parse(el.getAttribute('data-find') || '[]'); } catch (e) {}
    var title = el.querySelector('h2') ? el.querySelector('h2').textContent : '';
    var desc = el.querySelector('.tool-text p') ? el.querySelector('.tool-text p').textContent : '';
    return {
      el: el, order: i, title: fold(title), desc: fold(desc),
      phrases: phrases, folded: phrases.map(fold),
      found: el.querySelector('.tool-found')
    };
  });

  function run() {
    var raw = input.value.trim();
    var words = fold(raw).split(/[^a-z0-9\u00c0-\uffff]+/).filter(Boolean);
    var shown = 0;
    var ranked = cards.map(function (c) {
      if (!words.length) return { c: c, ok: true, rank: 1, hits: [] };
      var hits = [];
      var titleHits = 0;
      var ok = words.every(function (w) {
        if (c.title.indexOf(w) !== -1) { titleHits++; return true; }
        if (c.desc.indexOf(w) !== -1) return true;
        var any = false;
        c.folded.forEach(function (p, i) {
          if (p.indexOf(w) !== -1) {
            any = true;
            if (hits.indexOf(c.phrases[i]) === -1) hits.push(c.phrases[i]);
          }
        });
        return any;
      });
      return { c: c, ok: ok, rank: titleHits === words.length ? 0 : 1, hits: hits };
    });
    ranked.sort(function (a, b) { return a.rank - b.rank || a.c.order - b.c.order; });
    ranked.forEach(function (r) {
      var c = r.c;
      c.el.hidden = !r.ok;
      if (r.ok) shown++;
      grid.appendChild(c.el);
      if (r.ok && r.hits.length) {
        c.found.textContent = '';
        var label = document.createElement('b');
        label.textContent = 'Inside: ';
        c.found.appendChild(label);
        c.found.appendChild(document.createTextNode(r.hits.slice(0, 3).join(' · ') +
          (r.hits.length > 3 ? ' · +' + (r.hits.length - 3) : '')));
        c.found.hidden = false;
      } else {
        c.found.hidden = true;
      }
    });
    if (none) {
      none.hidden = !(words.length && shown === 0);
      if (noneQ) noneQ.textContent = raw;
    }
    if (count) count.textContent = words.length ? (shown === 1 ? '1 tool' : shown + ' tools') : '';
    var url = new URL(location.href);
    if (raw) url.searchParams.set('q', raw); else url.searchParams.delete('q');
    history.replaceState(null, '', url.pathname + url.search);
  }

  input.addEventListener('input', run);
  input.addEventListener('keydown', function (e) {
    if (e.key === 'Escape' && input.value) { e.preventDefault(); e.stopPropagation(); input.value = ''; run(); }
    if (e.key === 'Enter') {
      var first = cards.filter(function (c) { return !c.el.hidden; })
        .sort(function (a, b) { return Array.prototype.indexOf.call(grid.children, a.el) - Array.prototype.indexOf.call(grid.children, b.el); })[0];
      if (first) { e.preventDefault(); location.href = first.el.href; }
    }
  });
  // "Search everything" hands the query to the sidebar's palette.
  if (none) {
    var all = none.querySelector('[data-omni-open]');
    if (all) all.addEventListener('click', function () {
      var omni = document.getElementById('omni-input');
      if (!omni) return;
      omni.value = input.value.trim();
      setTimeout(function () { omni.dispatchEvent(new Event('input', { bubbles: true })); }, 0);
    });
  }
  var q = new URLSearchParams(location.search).get('q');
  if (q) { input.value = q; run(); }
})();
