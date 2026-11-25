/* Lantern: the header's shadow on scroll, the phone menu, the light/dark
 * switch, the homepage headline's italic tail, and "Expand all" on a
 * course's content. Every page works without it. */
(function () {
  function ready(fn) { if (document.readyState !== 'loading') { fn(); } else { document.addEventListener('DOMContentLoaded', fn); } }
  ready(function () {
    var header = document.querySelector('[data-header]');
    if (header) {
      var onScroll = function () { header.classList.toggle('is-scrolled', window.scrollY > 8); };
      window.addEventListener('scroll', onScroll, {passive: true});
      onScroll();
    }

    var toggle = document.querySelector('.menu-toggle'), nav = document.getElementById('site-nav');
    if (toggle && nav) {
      toggle.addEventListener('click', function () {
        var open = nav.classList.toggle('open');
        toggle.setAttribute('aria-expanded', open ? 'true' : 'false');
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
        try { localStorage.setItem('vuuut-scheme', next); } catch (e) { /* private mode */ }
      });
    }

    // "Learn the skills that light the way" → "…that <em>light the way</em>".
    document.querySelectorAll('[data-italic-tail]').forEach(function (h) {
      var words = h.textContent.trim().split(/\s+/);
      if (words.length < 4) { return; }
      var tail = words.splice(words.length - 2).join(' ');
      h.textContent = words.join(' ') + ' ';
      var em = document.createElement('em');
      em.textContent = tail;
      h.appendChild(em);
    });

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

    // Homepage proof: learners and average rating, summed from the courses.
    var proof = document.querySelector('[data-proof]');
    if (proof) {
      var learners = 0, sum = 0, n = 0;
      proof.querySelectorAll('[data-s]').forEach(function (i) {
        learners += parseInt(i.dataset.s || '0', 10) || 0;
        var r = parseFloat(i.dataset.r || ''), k = parseInt(i.dataset.n || '0', 10) || 0;
        if (r && k) { sum += r * k; n += k; }
      });
      var all = proof.querySelectorAll('[data-s]').length;
      proof.querySelector('[data-proof-students]').textContent = learners > 0 ? learners.toLocaleString() + '+' : 'New';
      proof.querySelector('[data-proof-courses]').textContent = all;
      if (n) { proof.querySelector('[data-proof-rating]').textContent = (sum / n).toFixed(1); } else { proof.querySelector('[data-proof-rate]').hidden = true; }
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
