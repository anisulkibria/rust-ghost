/* Stackline theme controls. */
(function () {
  function ready(fn) { if (document.readyState !== 'loading') { fn(); } else { document.addEventListener('DOMContentLoaded', fn); } }
  ready(function () {
    var toggle = document.querySelector('.menu-toggle'), nav = document.getElementById('site-nav');
    if (toggle && nav) {
      toggle.addEventListener('click', function () {
        var open = nav.classList.toggle('open');
        toggle.setAttribute('aria-expanded', open ? 'true' : 'false');
      });
      nav.addEventListener('keydown', function (event) {
        if (event.key === 'Escape' && nav.classList.contains('open')) {
          nav.classList.remove('open');
          toggle.setAttribute('aria-expanded', 'false');
          toggle.focus();
        }
      });
    }

    var scheme = document.querySelector('[data-scheme-toggle]');
    if (scheme) {
      scheme.addEventListener('click', function () {
        var root = document.documentElement;
        var now = root.getAttribute('data-scheme');
        var dark = now === 'dark' || (now === 'auto' && window.matchMedia('(prefers-color-scheme: dark)').matches);
        var next = dark ? 'light' : 'dark';
        root.setAttribute('data-scheme', next);
        try { localStorage.setItem('stackline-scheme', next); } catch (e) { /* private mode */ }
      });
    }

    // Instructor page: courses, students and the average rating, from the cards.
    var stats = document.querySelector('[data-ins-stats]');
    var cards = document.querySelectorAll('[data-ins-courses] .ins-card');
    if (stats && cards.length) {
      var students = 0, weighted = 0, reviews = 0;
      cards.forEach(function (c) {
        students += parseInt(c.dataset.students || '0', 10) || 0;
        var r = parseFloat(c.dataset.rating || ''), n = parseInt(c.dataset.reviews || '0', 10) || 0;
        if (r && n) { weighted += r * n; reviews += n; }
      });
      stats.querySelector('[data-sum=courses]').textContent = cards.length;
      stats.querySelector('[data-sum=students]').textContent = students.toLocaleString();
      stats.querySelector('[data-sum=rating]').textContent = reviews ? (weighted / reviews).toFixed(1) : '–';
      stats.hidden = false;
    }

    var expand = document.querySelector('[data-expand-all]');
    if (expand) {
      expand.addEventListener('click', function () {
        var all = document.querySelectorAll('.syl');
        var open = Array.prototype.some.call(all, function (d) { return !d.open; });
        all.forEach(function (d) { d.open = open; });
        expand.textContent = open ? 'Collapse all' : 'Expand all';
      });
    }
  });
})();
