/* The site's system surfaces, drawn in the admin's design: Portal (the
 * sticky button, signup, signin, the account modal and its toast) and Search.
 *
 * Both are Ghost's own apps, loaded from a CDN, and both render inside
 * iframes. Those iframes are same-origin (they have no `src`), so this page
 * may reach into them — which is what makes a restyle possible at all
 * without rewriting either app. This script finds them as they appear and
 * puts one stylesheet into each:
 *
 *   - `vuuut-portal.css` into every Portal iframe,
 *   - `vuuut-search.css` into Search's.
 *
 * It also hands each iframe two values it cannot work out for itself: the
 * site's accent colour and the theme's body typeface, so Portal speaks the
 * site's type rather than its own system stack.
 *
 * Why it re-checks: Portal is React, and it rewrites an iframe's document as
 * its popup changes screen. A style added once can be gone a moment later, so
 * each tracked iframe is looked at on a short timer while it exists. The check
 * is one `getElementById` — cheap enough to run four times a second.
 *
 * Fails quietly: if anything here breaks, the page keeps Portal and Search
 * exactly as Ghost draws them.
 */
(function () {
  'use strict';
  if (window.__vuuutSystem) return;
  window.__vuuutSystem = true;

  var me = document.currentScript;
  var src = (me && me.src) || '';
  var version = (src.split('?v=')[1] || '').split('&')[0];
  var base = src ? src.replace(/vuuut-system\.js.*$/, '') : '/public/';

  var accent = getComputedStyle(document.documentElement).getPropertyValue('--ghost-accent-color').trim() || '#3a3a3a';
  var font = '';
  try { font = getComputedStyle(document.body).fontFamily; } catch (e) {}

  var sheets = {};
  function sheet(name) {
    if (!sheets[name]) {
      sheets[name] = fetch(base + name + '?v=' + version)
        .then(function (r) { return r.ok ? r.text() : ''; })
        .catch(function () { return ''; });
    }
    return sheets[name];
  }

  /* Which stylesheet an iframe gets, if any. By container first — titles are
   * translated on a non-English site — and by Portal's fixed titles second. */
  var PORTAL_TITLES = { 'portal-trigger': 1, 'portal-popup': 1, 'portal-notification': 1 };
  function which(frame) {
    if (frame.closest('#ghost-portal-root') || PORTAL_TITLES[frame.title]) return 'vuuut-portal.css';
    if (frame.closest('#sodo-search-root')) return 'vuuut-search.css';
    return null;
  }

  function apply(frame, text) {
    var doc;
    try { doc = frame.contentDocument; } catch (e) { return; }
    if (!doc || !doc.head || !text) return;
    if (!doc.getElementById('vuuut-skin')) {
      var style = doc.createElement('style');
      style.id = 'vuuut-skin';
      style.textContent = text;
      doc.head.appendChild(style);
    }
    var root = doc.documentElement;
    if (root.style.getPropertyValue('--vu-accent') !== accent) {
      root.style.setProperty('--vu-accent', accent);
      if (font) root.style.setProperty('--vu-font', font);
    }
  }

  var tracked = [];
  var timer = null;
  function tick() {
    tracked = tracked.filter(function (t) { return t.frame.isConnected; });
    tracked.forEach(function (t) { sheet(t.name).then(function (text) { apply(t.frame, text); }); });
    if (!tracked.length && timer) { clearInterval(timer); timer = null; }
  }

  function scan() {
    var frames = document.querySelectorAll('iframe');
    for (var i = 0; i < frames.length; i++) {
      var frame = frames[i];
      if (frame.__vuuut) continue;
      var name = which(frame);
      if (!name) continue;
      frame.__vuuut = true;
      tracked.push({ frame: frame, name: name });
      frame.addEventListener('load', tick);
    }
    if (tracked.length) {
      tick();
      if (!timer) timer = setInterval(tick, 250);
    }
  }

  new MutationObserver(scan).observe(document.documentElement, { childList: true, subtree: true });
  scan();
})();
