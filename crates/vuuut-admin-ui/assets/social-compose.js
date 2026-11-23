/*
 * The share composer's character counters. Each draft carries its network's
 * limit and how many characters a link counts as there (X and Mastodon count
 * every link as 23). The server checks the same limit again before anything
 * is queued — this only saves a round trip.
 */
(function () {
  'use strict';
  function weigh(text, link) {
    if (!link) return Array.from(text).length;
    return text.split(/(\s+)/).reduce(function (n, w) {
      return n + (/^https?:\/\//.test(w) ? link : Array.from(w).length);
    }, 0);
  }
  Array.prototype.forEach.call(document.querySelectorAll('.sm-draft'), function (d) {
    var area = d.querySelector('textarea');
    var out = d.querySelector('[data-count]');
    var limit = +d.getAttribute('data-limit') || 0;
    var link = +d.getAttribute('data-link') || 0;
    if (!area || !out) return;
    function update() {
      var n = weigh(area.value, link);
      out.textContent = String(n);
      d.classList.toggle('sm-over', n > limit);
    }
    area.addEventListener('input', update);
    update();
  });
})();
