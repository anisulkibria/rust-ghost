/* The portfolio's small conveniences. Every page works without it.
 *
 *   .vf-chip[data-filter]   — filter the grid in place (the link still goes
 *                             to the discipline's own page without script)
 *   [data-vf-gallery] a     — open pictures in a lightbox, ← → to move
 *   [data-vf-print]         — print the résumé
 *   [data-vf-contact]       — gather the ticked services into one field
 *
 * Nothing from the page is placed as HTML; the lightbox uses the link's own
 * href as an image source and nothing else.
 */
(function () {
  'use strict';
  function ready(fn) { if (document.readyState === 'loading') { document.addEventListener('DOMContentLoaded', fn); } else { fn(); } }


  // Some themes put page content in a column that is not centred on the
  // page; the wide layout centres on the column, so nudge it back to the
  // middle of the viewport.
  function centre(sel) {
    function run() {
      document.querySelectorAll(sel).forEach(function (el) {
        el.style.marginLeft = '';
        var r = el.getBoundingClientRect();
        var off = (document.documentElement.clientWidth - r.width) / 2 - r.left;
        if (Math.abs(off) > 1) { el.style.marginLeft = off + 'px'; }
      });
    }
    run();
    window.addEventListener('resize', run);
  }
  ready(function () {
    centre('.vf');
    // filters
    var grid = document.querySelector('[data-vf-grid]');
    var chips = document.querySelectorAll('.vf-chip[data-filter]');
    if (grid && chips.length) {
      chips.forEach(function (chip) {
        chip.addEventListener('click', function (e) {
          if (e.metaKey || e.ctrlKey || e.shiftKey) { return; }
          e.preventDefault();
          var f = chip.getAttribute('data-filter');
          chips.forEach(function (c) { c.classList.toggle('on', c === chip); });
          grid.querySelectorAll('.vf-card').forEach(function (card) {
            var cats = (card.getAttribute('data-cats') || '').split(' ');
            card.hidden = !!f && cats.indexOf(f) < 0;
          });
          if (history.replaceState) { history.replaceState(null, '', chip.getAttribute('href')); }
        });
      });
    }

    // lightbox
    document.querySelectorAll('[data-vf-gallery]').forEach(function (g) {
      var links = Array.prototype.slice.call(g.querySelectorAll('a[data-vf-zoom]'));
      links.forEach(function (a, i) {
        a.addEventListener('click', function (e) { e.preventDefault(); open(links, i); });
      });
    });
    function open(links, i) {
      var box = document.createElement('div');
      box.className = 'vf-lightbox';
      box.setAttribute('role', 'dialog');
      box.setAttribute('aria-label', 'Picture');
      var img = document.createElement('img');
      img.alt = '';
      var prev = document.createElement('button'); prev.className = 'vf-lb-prev'; prev.type = 'button'; prev.textContent = '‹'; prev.setAttribute('aria-label', 'Previous');
      var next = document.createElement('button'); next.className = 'vf-lb-next'; next.type = 'button'; next.textContent = '›'; next.setAttribute('aria-label', 'Next');
      var close = document.createElement('button'); close.className = 'vf-lb-close'; close.type = 'button'; close.textContent = '✕'; close.setAttribute('aria-label', 'Close');
      function show(n) { i = (n + links.length) % links.length; img.src = links[i].getAttribute('href'); }
      function shut() { document.removeEventListener('keydown', key); box.remove(); links[i].focus(); }
      function key(e) { if (e.key === 'Escape') { shut(); } else if (e.key === 'ArrowLeft') { show(i - 1); } else if (e.key === 'ArrowRight') { show(i + 1); } }
      prev.addEventListener('click', function (e) { e.stopPropagation(); show(i - 1); });
      next.addEventListener('click', function (e) { e.stopPropagation(); show(i + 1); });
      close.addEventListener('click', shut);
      box.addEventListener('click', function (e) { if (e.target === box || e.target === img) { shut(); } });
      document.addEventListener('keydown', key);
      box.appendChild(img);
      if (links.length > 1) { box.appendChild(prev); box.appendChild(next); }
      box.appendChild(close);
      document.body.appendChild(box);
      show(i);
      close.focus();
    }

    // print
    document.querySelectorAll('[data-vf-print]').forEach(function (b) { b.addEventListener('click', function () { window.print(); }); });

    // services → one field
    document.querySelectorAll('[data-vf-contact]').forEach(function (form) {
      form.addEventListener('submit', function () {
        var picked = Array.prototype.map.call(form.querySelectorAll('[data-vf-service]:checked'), function (c) { return c.value; });
        var hidden = form.querySelector('[data-vf-services]');
        if (hidden) { hidden.value = picked.join(','); }
      });
    });
  });
})();
