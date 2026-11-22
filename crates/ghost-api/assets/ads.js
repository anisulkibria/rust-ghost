/*
 * Vuuut ads — counting what readers actually saw.
 *
 * The server already chose and placed every ad on this page. This script
 * only reports back:
 *
 *   - an impression, once at least half of an ad has been on screen for one
 *     continuous second (the IAB viewability rule), counted once per ad per
 *     page view;
 *   - a click, when a link inside an ad is followed. The link itself is an
 *     ordinary link — nothing is redirected or rewritten.
 *
 * Reports are batched and sent with sendBeacon, so a click never waits on
 * the network. The server counts only ids of live ads and ignores the rest;
 * this script is a convenience, never the authority. It sets no cookie and
 * stores nothing except, for the sticky bar, that it was closed this session.
 */
(function () {
  'use strict';
  var script = document.currentScript;
  var endpoint = script && script.getAttribute('data-ads');
  if (!endpoint) return;

  var seen = {}, queue = { i: [], c: [] }, timer = null;

  function flush() {
    timer = null;
    if (!queue.i.length && !queue.c.length) return;
    var body = 'i=' + encodeURIComponent(queue.i.join(',')) + '&c=' + encodeURIComponent(queue.c.join(','));
    queue = { i: [], c: [] };
    var blob = new Blob([body], { type: 'application/x-www-form-urlencoded' });
    if (!(navigator.sendBeacon && navigator.sendBeacon(endpoint, blob))) {
      try {
        fetch(endpoint, { method: 'POST', body: body, keepalive: true, credentials: 'same-origin',
          headers: { 'Content-Type': 'application/x-www-form-urlencoded' } }).catch(function () {});
      } catch (e) {}
    }
  }

  function report(kind, id) {
    if (kind === 'i') { if (seen[id]) return; seen[id] = true; }
    queue[kind].push(id);
    if (!timer) timer = setTimeout(flush, 1500);
  }

  var ads = Array.prototype.slice.call(document.querySelectorAll('[data-ad]'));

  // Viewability: half on screen for a continuous second.
  if ('IntersectionObserver' in window) {
    var clocks = {};
    var io = new IntersectionObserver(function (entries) {
      entries.forEach(function (e) {
        var id = e.target.getAttribute('data-ad');
        if (e.isIntersecting && e.intersectionRatio >= 0.5) {
          if (!clocks[id]) clocks[id] = setTimeout(function () { report('i', id); io.unobserve(e.target); }, 1000);
        } else if (clocks[id]) {
          clearTimeout(clocks[id]);
          clocks[id] = null;
        }
      });
    }, { threshold: [0, 0.5, 1] });
    ads.forEach(function (a) { if (a.offsetParent !== null || a.closest('.vad-sticky')) io.observe(a); });
  }

  document.addEventListener('click', function (e) {
    var link = e.target.closest && e.target.closest('[data-ad] a[href]');
    if (!link) return;
    report('c', link.closest('[data-ad]').getAttribute('data-ad'));
    flush();
  }, true);

  // The sticky bar can be closed, and stays closed for the session.
  var sticky = document.querySelector('[data-ad-sticky]');
  if (sticky) {
    try { if (sessionStorage.getItem('vad-sticky-closed')) sticky.remove(); } catch (e) {}
    var close = sticky.querySelector('.vad-close');
    if (close) close.addEventListener('click', function () {
      sticky.remove();
      try { sessionStorage.setItem('vad-sticky-closed', '1'); } catch (e) {}
    });
  }

  window.addEventListener('pagehide', flush);
  document.addEventListener('visibilitychange', function () { if (document.visibilityState === 'hidden') flush(); });
})();
