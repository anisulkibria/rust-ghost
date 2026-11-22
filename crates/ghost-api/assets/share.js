/*
 * Share buttons — the one thing they need a script for: "Copy link".
 * Every other button is a plain link to the network's own share page.
 * Nothing here talks to any network, stores anything, or sets a cookie.
 */
(function () {
  'use strict';
  document.addEventListener('click', function (e) {
    var b = e.target.closest && e.target.closest('.vsb-copy[data-copy]');
    if (!b) return;
    var url = b.getAttribute('data-copy');
    var done = function () {
      var label = b.textContent;
      b.textContent = 'Copied';
      b.classList.add('vsb-done');
      setTimeout(function () { b.textContent = label; b.classList.remove('vsb-done'); }, 1800);
    };
    if (navigator.clipboard && navigator.clipboard.writeText) {
      navigator.clipboard.writeText(url).then(done, function () { window.prompt('Copy this link', url); });
    } else {
      window.prompt('Copy this link', url);
    }
  });
})();
