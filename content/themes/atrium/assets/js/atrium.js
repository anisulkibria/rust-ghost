/* Atrium: the header's line on scroll, the phone menu, light/dark, and the
 * homepage headline's italic tail. Every page works without it. */
(function () {
  function ready(fn) { if (document.readyState !== 'loading') { fn(); } else { document.addEventListener('DOMContentLoaded', fn); } }
  ready(function () {
    var hd = document.querySelector('[data-header]');
    if (hd) { var on = function () { hd.classList.toggle('scrolled', window.scrollY > 8); }; window.addEventListener('scroll', on, {passive: true}); on(); }
    var m = document.querySelector('.menu'), nav = document.getElementById('nav');
    if (m && nav) { m.addEventListener('click', function () { var o = nav.classList.toggle('open'); m.setAttribute('aria-expanded', o ? 'true' : 'false'); }); }
    var t = document.querySelector('[data-scheme-toggle]');
    if (t) {
      t.addEventListener('click', function () {
        var r = document.documentElement, now = r.getAttribute('data-scheme');
        var dark = now === 'dark' || (now === 'auto' && window.matchMedia('(prefers-color-scheme: dark)').matches);
        r.setAttribute('data-scheme', dark ? 'light' : 'dark');
        try { localStorage.setItem('vuuut-scheme', dark ? 'light' : 'dark'); } catch (e) { /* private mode */ }
      });
    }
    // The last N words of the headline in italics.
    document.querySelectorAll('[data-italic-tail]').forEach(function (h) {
      var n = parseInt(h.getAttribute('data-italic-tail'), 10) || 2;
      var words = h.textContent.trim().split(/\s+/);
      if (words.length <= n) { return; }
      var tail = words.splice(words.length - n).join(' ');
      h.textContent = words.join(' ') + ' ';
      var em = document.createElement('em'); em.textContent = tail; h.appendChild(em);
    });
  });
})();
