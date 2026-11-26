/* Earshot: the header, the mobile drawer and the waveforms.
 * Playing is podcast.js's job (Tools → Podcast); this only draws.
 */
(function () {
  'use strict';
  function ready(fn) { if (document.readyState === 'loading') { document.addEventListener('DOMContentLoaded', fn); } else { fn(); } }

  // A waveform for an episode, the same every time for the same episode.
  function seeded(str) {
    var h = 2166136261;
    for (var i = 0; i < str.length; i++) { h ^= str.charCodeAt(i); h = Math.imul(h, 16777619); }
    return function () { h ^= h << 13; h ^= h >>> 17; h ^= h << 5; return ((h >>> 0) % 1000) / 1000; };
  }
  function wave(el) {
    var rnd = seeded(el.getAttribute('data-ea-wave') || 'x');
    var n = 96, w = 4, gap = 2, H = 52, rects = '';
    var prev = .5;
    for (var i = 0; i < n; i++) {
      var shape = Math.sin(i / n * Math.PI) * .55 + .35;
      var v = Math.max(.12, Math.min(1, (prev * .45 + rnd() * .55) * shape * 1.35));
      prev = v;
      var h = Math.round(v * H);
      rects += '<rect x="' + (i * (w + gap)) + '" y="' + ((H - h) / 2) + '" width="' + w + '" height="' + h + '" rx="2"/>';
    }
    var svg = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ' + (n * (w + gap) - gap) + ' ' + H + '" preserveAspectRatio="none">' + rects + '</svg>';
    el.style.setProperty('--ea-wave', 'url("data:image/svg+xml,' + encodeURIComponent(svg) + '")');
    el.classList.add('is-drawn');
  }

  ready(function () {
    document.querySelectorAll('[data-ea-wave]').forEach(wave);

    var head = document.querySelector('[data-ea-head]');
    if (head) {
      var stick = function () { head.classList.toggle('is-stuck', window.scrollY > 10); };
      stick();
      window.addEventListener('scroll', stick, { passive: true });
    }

    var drawer = document.querySelector('[data-ea-drawer]');
    var burger = document.querySelector('[data-ea-burger]');
    if (drawer && burger) {
      var close = drawer.querySelector('[data-ea-drawer-close]');
      burger.addEventListener('click', function () {
        drawer.showModal();
        burger.setAttribute('aria-expanded', 'true');
      });
      close.addEventListener('click', function () { drawer.close(); });
      drawer.addEventListener('click', function (event) {
        if (event.target.closest('a')) drawer.close();
      });
      drawer.addEventListener('close', function () {
        burger.setAttribute('aria-expanded', 'false');
        burger.focus();
      });
    }
  });
})();
