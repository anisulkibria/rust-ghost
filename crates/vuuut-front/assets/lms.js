/* The public LMS: small comforts on top of pages that work without it.
 *  - the lesson player's course outline opens as a drawer on phones;
 *  - a timed quiz counts down and hands itself in when time is up;
 *  - choosing answers marks the question as answered;
 *  - finishing a course throws a little confetti. */
(function () {
  function ready(fn) { if (document.readyState !== 'loading') { fn(); } else { document.addEventListener('DOMContentLoaded', fn); } }
  ready(function () {
    var toggle = document.querySelector('.vl-side-toggle'), side = document.getElementById('vl-side');
    if (toggle && side) {
      toggle.addEventListener('click', function () {
        var open = side.classList.toggle('open');
        toggle.setAttribute('aria-expanded', open ? 'true' : 'false');
      });
      side.addEventListener('click', function (e) { if (e.target === side) { side.classList.remove('open'); } });
    }
    var cur = document.querySelector('.vl-side .current');
    if (cur && cur.scrollIntoView) { cur.scrollIntoView({block: 'center'}); }

    document.querySelectorAll('form.vl-quiz').forEach(function (f) {
      f.addEventListener('change', function (e) {
        var q = e.target.closest('.vl-q');
        if (q) { q.classList.toggle('answered', !!q.querySelector('input:checked') || !!(q.querySelector('input.vl-in') || {}).value); }
      });
      var limit = parseInt(f.dataset.limit || '0', 10), clock = f.querySelector('.vl-timer');
      if (limit > 0 && clock) {
        var end = Date.now() + limit * 60000;
        var tick = function () {
          var left = Math.max(0, Math.round((end - Date.now()) / 1000));
          clock.textContent = Math.floor(left / 60) + ':' + String(left % 60).padStart(2, '0');
          clock.classList.toggle('low', left < 60);
          if (left === 0) { f.submit(); return; }
          setTimeout(tick, 1000);
        };
        tick();
      }
      f.addEventListener('submit', function (e) {
        var empty = Array.prototype.filter.call(f.querySelectorAll('.vl-q'), function (q) { return !q.querySelector('input:checked') && !((q.querySelector('input.vl-in') || {}).value); });
        if (empty.length && !f.dataset.sure && !window.confirm(empty.length + ' question' + (empty.length > 1 ? 's are' : ' is') + ' unanswered. Submit anyway?')) { e.preventDefault(); return; }
        f.dataset.sure = '1';
      });
    });

    if (document.querySelector('.vl-celebrate') && !window.matchMedia('(prefers-reduced-motion: reduce)').matches) {
      var colours = ['#ff6b6b', '#ffd93d', '#6bcB77', '#4d96ff', '#c77dff', '#ff9f1c'];
      var layer = document.createElement('div');
      layer.className = 'vl-confetti';
      for (var i = 0; i < 90; i++) {
        var p = document.createElement('i');
        p.style.left = Math.random() * 100 + 'vw';
        p.style.background = colours[i % colours.length];
        p.style.animationDelay = (Math.random() * 0.8) + 's';
        p.style.transform = 'rotate(' + Math.random() * 360 + 'deg)';
        layer.appendChild(p);
      }
      document.body.appendChild(layer);
      setTimeout(function () { layer.remove(); }, 4200);
    }
  });
})();
