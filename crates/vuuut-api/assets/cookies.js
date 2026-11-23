/*
 * Vuuut cookie consent — the visitor's half.
 *
 * The server has already held back every recognised tracker in this page
 * (`<script type="text/plain" data-cookie-category="…">`, and embeds with
 * `data-cookie-src`). This script decides which of them come back, because
 * that is the visitor's decision, and it does three things with it:
 *
 *   1. remembers it in a first-party cookie, `vuuut_consent`, for as long as
 *      the site's setting allows, and for the current banner revision only;
 *   2. re-enables the held-back scripts and embeds for the allowed categories,
 *      in document order, and tells the integrations that listen (Vuuut Click,
 *      Google's consent mode, anyone on the `vuuut:consent` event);
 *   3. posts the decision to the site's consent log — a random id this browser
 *      generated, the choice and the page, nothing else.
 *
 * It is a convenience for the visitor, never an authority: the server
 * rebuilds and bounds everything it logs. Text from the configuration is
 * placed with textContent only.
 *
 * Public API, for theme authors: window.vuuutConsent.open(),
 * .allowed('analytics'), .get(), .on(fn). Any link to `#cookie-settings`, or
 * any element with `data-cookie-settings`, opens the choices.
 */
(function () {
  'use strict';

  var script = document.currentScript;
  if (!script) return;
  var cfg;
  try { cfg = JSON.parse(script.getAttribute('data-config') || '{}'); } catch (e) { return; }
  if (!cfg || !cfg.categories) return;

  var COOKIE = 'vuuut_consent';
  var OPTIONAL = [];
  for (var i = 0; i < cfg.categories.length; i++) {
    if (!cfg.categories[i].required) OPTIONAL.push(cfg.categories[i].key);
  }
  var listeners = [];
  var state = null;      // { id, granted: {key: true}, revision, at }
  var activated = {};    // categories whose scripts have already been run
  var banner = null;
  var dialog = null;
  var reopen = null;

  // ---------------------------------------------------------------- storage

  function uuid() {
    if (window.crypto && crypto.randomUUID) return crypto.randomUUID();
    var b = new Uint8Array(16);
    (window.crypto || window.msCrypto).getRandomValues(b);
    b[6] = (b[6] & 15) | 64; b[8] = (b[8] & 63) | 128;
    var h = '';
    for (var k = 0; k < 16; k++) {
      h += (b[k] + 256).toString(16).slice(1);
      if (k === 3 || k === 5 || k === 7 || k === 9) h += '-';
    }
    return h;
  }

  function read() {
    var parts = document.cookie ? document.cookie.split('; ') : [];
    for (var k = 0; k < parts.length; k++) {
      if (parts[k].indexOf(COOKIE + '=') !== 0) continue;
      try {
        var v = JSON.parse(decodeURIComponent(parts[k].slice(COOKIE.length + 1)));
        if (!v || v.r !== cfg.revision || typeof v.id !== 'string') return null;
        var granted = { necessary: true };
        (v.c || '').split(',').forEach(function (c) { if (c) granted[c] = true; });
        return { id: v.id, granted: granted, action: v.a || 'custom', at: v.t || 0 };
      } catch (e) { return null; }
    }
    return null;
  }

  function write(s) {
    var value = encodeURIComponent(JSON.stringify({
      id: s.id, r: cfg.revision, c: list(s.granted), a: s.action, t: Date.now()
    }));
    var secure = location.protocol === 'https:' ? '; Secure' : '';
    document.cookie = COOKIE + '=' + value + '; Path=/; Max-Age=' + (cfg.days * 86400) +
      '; SameSite=Lax' + secure;
  }

  function list(granted) {
    var out = ['necessary'];
    OPTIONAL.forEach(function (k) { if (granted[k]) out.push(k); });
    return out.join(',');
  }

  function log(s) {
    if (!cfg.log || !cfg.endpoint) return;
    var body = 'action=' + encodeURIComponent(s.action) +
      '&categories=' + encodeURIComponent(list(s.granted)) +
      '&id=' + encodeURIComponent(s.id) +
      '&page=' + encodeURIComponent(location.pathname);
    try {
      fetch(cfg.endpoint, {
        method: 'POST', credentials: 'same-origin', keepalive: true,
        headers: { 'Content-Type': 'application/x-www-form-urlencoded' }, body: body
      }).catch(function () {});
    } catch (e) {}
  }

  // ----------------------------------------------------------- the decision

  function gpcOn() {
    return cfg.gpc && navigator.globalPrivacyControl === true;
  }

  function decide(action, granted) {
    var previous = state;
    granted.necessary = true;
    // GPC is an opt-out of sale and sharing, whatever was clicked.
    if (gpcOn()) granted.marketing = false;
    state = { id: previous ? previous.id : uuid(), granted: granted, action: action, at: Date.now() };
    write(state);
    log(state);
    // Withdrawing something already running cannot be done to a script that
    // has run; clear what it stored and start the page again without it.
    var withdrawn = OPTIONAL.some(function (k) { return activated[k] && !granted[k]; });
    if (withdrawn) {
      OPTIONAL.forEach(function (k) { if (!granted[k]) forget(k); });
      announce();
      location.reload();
      return;
    }
    apply();
    hideBanner();
    closeDialog();
    showReopen();
  }

  function apply() {
    announce();
    var allowed = OPTIONAL.filter(function (k) { return state.granted[k]; });
    allowed.forEach(function (k) { activated[k] = true; });
    runScripts(allowed);
    loadEmbeds();
  }

  /** Tell everything that listens. */
  function announce() {
    var detail = { id: state.id, categories: list(state.granted).split(','), action: state.action };
    // Vuuut Click waits for exactly this call in consent-required mode.
    var click = function () {
      if (window.vuuutClick && typeof window.vuuutClick.consent === 'function') {
        window.vuuutClick.consent(!!state.granted.analytics);
        return true;
      }
      return false;
    };
    if (!click()) window.addEventListener('load', click, { once: true });
    // Google's consent mode, for sites that load gtag themselves.
    if (typeof window.gtag === 'function') {
      var g = function (b) { return b ? 'granted' : 'denied'; };
      window.gtag('consent', 'update', {
        analytics_storage: g(state.granted.analytics),
        ad_storage: g(state.granted.marketing),
        ad_user_data: g(state.granted.marketing),
        ad_personalization: g(state.granted.marketing),
        functionality_storage: g(state.granted.preferences),
        personalization_storage: g(state.granted.preferences)
      });
    }
    try { document.dispatchEvent(new CustomEvent('vuuut:consent', { detail: detail })); } catch (e) {}
    listeners.forEach(function (fn) { try { fn(detail); } catch (e) {} });
  }

  /**
   * Swap each held-back script for a live copy, one at a time in document
   * order: an inline snippet often calls the library the tag before it loads.
   */
  function runScripts(allowed) {
    var held = Array.prototype.slice.call(
      document.querySelectorAll('script[type="text/plain"][data-cookie-category]'));
    var queue = held.filter(function (s) {
      return allowed.indexOf(s.getAttribute('data-cookie-category')) !== -1;
    });
    (function next() {
      var old = queue.shift();
      if (!old) return;
      var live = document.createElement('script');
      for (var a = 0; a < old.attributes.length; a++) {
        var at = old.attributes[a];
        if (at.name === 'type' || at.name === 'data-cookie-category') continue;
        live.setAttribute(at.name, at.value);
      }
      if (old.getAttribute('data-cookie-type') === 'module') live.type = 'module';
      if (old.src) {
        live.onload = live.onerror = next;
        old.parentNode.replaceChild(live, old);
      } else {
        live.text = old.text;
        old.parentNode.replaceChild(live, old);
        next();
      }
    })();
  }

  /** Embeds: load the allowed ones, put a placeholder over the rest. */
  function loadEmbeds() {
    var frames = document.querySelectorAll('iframe[data-cookie-src]');
    Array.prototype.forEach.call(frames, function (f) {
      var cat = f.getAttribute('data-cookie-category') || 'marketing';
      if (state && state.granted[cat]) {
        load(f);
      } else if (!f.previousElementSibling || !f.previousElementSibling.classList ||
                 !f.previousElementSibling.classList.contains('vc-embed')) {
        placeholder(f, cat);
      }
    });
  }

  function load(f) {
    var ph = f.previousElementSibling;
    if (ph && ph.classList && ph.classList.contains('vc-embed')) ph.parentNode.removeChild(ph);
    f.style.display = '';
    f.setAttribute('src', f.getAttribute('data-cookie-src'));
    f.removeAttribute('data-cookie-src');
  }

  function placeholder(f, cat) {
    var vendor = f.getAttribute('data-cookie-vendor') || 'another site';
    var box = el('div', 'vc-embed');
    var w = f.getAttribute('width'), h = f.getAttribute('height');
    if (w && h && +w > 0) box.style.aspectRatio = w + ' / ' + h;
    box.appendChild(el('p', 'vc-embed-title', vendor.replace(/ embeds$/, '') + ' content'));
    box.appendChild(el('p', 'vc-embed-text',
      'This is hosted by ' + host(f.getAttribute('data-cookie-src')) +
      ', which sets cookies when it loads.'));
    var row = el('div', 'vc-embed-actions');
    var once = button('Load it once', 'vc-btn vc-btn-tonal', function () { load(f); });
    var always = button('Always allow', 'vc-btn vc-btn-text', function () {
      var g = copy(state ? state.granted : {});
      g[cat] = true;
      decide('custom', g);
    });
    row.appendChild(once); row.appendChild(always);
    box.appendChild(row);
    f.style.display = 'none';
    f.parentNode.insertBefore(box, f);
  }

  function host(url) {
    try { return new URL(url, location.href).hostname.replace(/^www\./, ''); } catch (e) { return 'another site'; }
  }

  /** Expire every declared cookie of a category on this host and its parents. */
  function forget(cat) {
    var names = [];
    cfg.categories.forEach(function (c) {
      if (c.key === cat) c.cookies.forEach(function (k) { names.push(k.name); });
    });
    var present = document.cookie ? document.cookie.split('; ').map(function (p) { return p.split('=')[0]; }) : [];
    var doomed = present.filter(function (n) {
      return names.some(function (d) {
        var star = d.indexOf('*');
        return star === -1 ? d === n : n.indexOf(d.slice(0, star)) === 0;
      });
    });
    // Trackers set their cookies on the widest domain they can, so try this
    // host and each parent down to (not including) the bare suffix.
    var parts = location.hostname.split('.');
    doomed.forEach(function (n) {
      document.cookie = n + '=; Max-Age=0; Path=/';
      for (var k = 0; k < parts.length - 1; k++) {
        document.cookie = n + '=; Max-Age=0; Path=/; Domain=' + parts.slice(k).join('.');
      }
    });
  }

  // ------------------------------------------------------------------ DOM

  function el(tag, cls, text) {
    var e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text != null) e.textContent = text;
    return e;
  }

  function button(text, cls, fn) {
    var b = el('button', cls, text);
    b.type = 'button';
    b.addEventListener('click', fn);
    return b;
  }

  function copy(o) { var r = {}; for (var k in o) if (o.hasOwnProperty(k)) r[k] = o[k]; return r; }

  function all(value) {
    var g = { necessary: true };
    OPTIONAL.forEach(function (k) { g[k] = value; });
    return g;
  }

  function policyLink(parent) {
    if (!cfg.policy) return;
    var a = el('a', 'vc-link', 'Privacy policy');
    a.href = cfg.policy;
    parent.appendChild(a);
  }

  function showBanner() {
    if (banner) return;
    var modal = cfg.layout === 'modal';
    banner = el('div', 'vc-banner vc-' + cfg.layout);
    banner.setAttribute('role', modal ? 'alertdialog' : 'region');
    banner.setAttribute('aria-label', cfg.title);
    if (modal) banner.setAttribute('aria-modal', 'true');
    var card = el('div', 'vc-card');
    var text = el('div', 'vc-text');
    text.appendChild(el('h2', 'vc-title', cfg.title));
    var p = el('p', 'vc-message', cfg.message + ' ');
    policyLink(p);
    text.appendChild(p);
    card.appendChild(text);

    var actions = el('div', 'vc-actions');
    if (cfg.mode === 'notice') {
      actions.appendChild(button('Settings', 'vc-btn vc-btn-text', openDialog));
      actions.appendChild(button('OK', 'vc-btn vc-btn-filled', function () { decide('acknowledge', all(true)); }));
    } else if (cfg.mode === 'opt_out') {
      actions.appendChild(button('Do not sell or share my information', 'vc-btn vc-btn-text', function () {
        var g = all(true); g.marketing = false; decide('custom', g);
      }));
      actions.appendChild(button('Choose', 'vc-btn vc-btn-tonal', openDialog));
      actions.appendChild(button('OK', 'vc-btn vc-btn-filled', function () { decide('accept_all', all(true)); }));
    } else {
      // Refusing is exactly as easy as agreeing: same size, same row, same
      // number of clicks. Regulators check.
      actions.appendChild(button('Choose', 'vc-btn vc-btn-text', openDialog));
      actions.appendChild(button('Reject all', 'vc-btn vc-btn-tonal', function () { decide('reject_all', all(false)); }));
      actions.appendChild(button('Accept all', 'vc-btn vc-btn-tonal', function () { decide('accept_all', all(true)); }));
    }
    card.appendChild(actions);
    banner.appendChild(card);
    document.body.appendChild(banner);
    hideReopen();
    requestAnimationFrame(function () { banner.classList.add('vc-in'); });
    if (modal) {
      var first = actions.querySelector('button');
      if (first) first.focus();
    }
  }

  function hideBanner() {
    if (!banner) return;
    var b = banner;
    banner = null;
    b.classList.remove('vc-in');
    setTimeout(function () { if (b.parentNode) b.parentNode.removeChild(b); }, 250);
  }

  function openDialog() {
    if (dialog) return;
    var current = state ? copy(state.granted) : (cfg.mode === 'opt_in' ? all(false) : all(true));
    var back = el('div', 'vc-scrim');
    var box = el('div', 'vc-dialog');
    box.setAttribute('role', 'dialog');
    box.setAttribute('aria-modal', 'true');
    box.setAttribute('aria-labelledby', 'vc-dialog-title');
    var head = el('div', 'vc-dialog-head');
    var h = el('h2', 'vc-title', 'Cookie choices');
    h.id = 'vc-dialog-title';
    head.appendChild(h);
    head.appendChild(button('×', 'vc-close', closeDialog));
    head.lastChild.setAttribute('aria-label', 'Close');
    box.appendChild(head);
    var intro = el('p', 'vc-message', 'Choose what this site may store on your device. Strictly necessary cookies are always on. ');
    policyLink(intro);
    box.appendChild(intro);

    var inputs = {};
    var body = el('div', 'vc-cats');
    cfg.categories.forEach(function (c) {
      var item = el('div', 'vc-cat');
      var top = el('label', 'vc-cat-top');
      var words = el('span', 'vc-cat-words');
      words.appendChild(el('span', 'vc-cat-label', c.label));
      words.appendChild(el('span', 'vc-cat-desc', c.description));
      top.appendChild(words);
      var sw = el('span', 'vc-switch');
      var input = el('input');
      input.type = 'checkbox';
      input.checked = c.required || !!current[c.key];
      input.disabled = !!c.required;
      input.setAttribute('aria-label', c.label);
      inputs[c.key] = input;
      sw.appendChild(input);
      sw.appendChild(el('span', 'vc-track'));
      top.appendChild(sw);
      item.appendChild(top);
      if (c.cookies && c.cookies.length) {
        var det = el('details', 'vc-cookies');
        det.appendChild(el('summary', null, c.cookies.length === 1 ? '1 cookie' : c.cookies.length + ' cookies'));
        det.appendChild(table(c.cookies));
        item.appendChild(det);
      }
      body.appendChild(item);
    });
    box.appendChild(body);

    if (state) {
      box.appendChild(el('p', 'vc-id', 'Your consent ID: ' + state.id));
    }
    var actions = el('div', 'vc-actions');
    actions.appendChild(button('Reject all', 'vc-btn vc-btn-text', function () { decide('reject_all', all(false)); }));
    actions.appendChild(button('Accept all', 'vc-btn vc-btn-tonal', function () { decide('accept_all', all(true)); }));
    actions.appendChild(button('Save choices', 'vc-btn vc-btn-filled', function () {
      var g = { necessary: true };
      OPTIONAL.forEach(function (k) { g[k] = inputs[k] && inputs[k].checked; });
      decide('custom', g);
    }));
    box.appendChild(actions);
    back.appendChild(box);
    back.addEventListener('click', function (e) { if (e.target === back) closeDialog(); });
    back.addEventListener('keydown', function (e) {
      if (e.key === 'Escape') { e.preventDefault(); closeDialog(); }
    });
    document.body.appendChild(back);
    dialog = back;
    requestAnimationFrame(function () { back.classList.add('vc-in'); });
    var focus = box.querySelector('input:not([disabled])') || box.querySelector('button');
    if (focus) focus.focus();
  }

  function closeDialog() {
    if (!dialog) return;
    var d = dialog;
    dialog = null;
    d.classList.remove('vc-in');
    setTimeout(function () { if (d.parentNode) d.parentNode.removeChild(d); }, 200);
  }

  function table(cookies) {
    var t = el('table', 'vc-table');
    var head = el('tr');
    ['Name', 'Provider', 'Kept for', 'Purpose'].forEach(function (h) { head.appendChild(el('th', null, h)); });
    var thead = el('thead'); thead.appendChild(head); t.appendChild(thead);
    var tb = el('tbody');
    cookies.forEach(function (k) {
      var r = el('tr');
      var name = el('td'); name.appendChild(el('code', null, k.name)); r.appendChild(name);
      r.appendChild(el('td', null, k.provider || ''));
      r.appendChild(el('td', null, k.duration || ''));
      r.appendChild(el('td', null, k.purpose || ''));
      tb.appendChild(r);
    });
    t.appendChild(tb);
    return t;
  }

  /** `<div data-cookie-declaration></div>` in a post becomes the full table. */
  function declarations() {
    var spots = document.querySelectorAll('[data-cookie-declaration]');
    Array.prototype.forEach.call(spots, function (spot) {
      spot.textContent = '';
      spot.classList.add('vc-declaration');
      cfg.categories.forEach(function (c) {
        if (!c.cookies || !c.cookies.length) return;
        spot.appendChild(el('h3', null, c.label));
        spot.appendChild(el('p', null, c.description));
        spot.appendChild(table(c.cookies));
      });
      spot.appendChild(button('Change your cookie choices', 'vc-btn vc-btn-tonal', openDialog));
    });
  }

  function showReopen() {
    if (!cfg.reopen || reopen) return;
    reopen = button('', 'vc-reopen', openDialog);
    reopen.setAttribute('aria-label', 'Cookie choices');
    reopen.title = 'Cookie choices';
    // A cookie with a bite out of it. Built with DOM calls, not markup.
    var ns = 'http://www.w3.org/2000/svg';
    var svg = document.createElementNS(ns, 'svg');
    svg.setAttribute('viewBox', '0 0 24 24');
    svg.setAttribute('aria-hidden', 'true');
    var path = document.createElementNS(ns, 'path');
    path.setAttribute('d', 'M21.95 10.99c-1.79-.03-3.7-1.95-2.68-4.22-2.98 1-5.77-1.59-5.19-4.56C6.95.71 2 6.58 2 12c0 5.52 4.48 10 10 10 5.89 0 10.54-5.08 9.95-11.01zM8.5 15c-.83 0-1.5-.67-1.5-1.5S7.67 12 8.5 12s1.5.67 1.5 1.5S9.33 15 8.5 15zm2-5C9.67 10 9 9.33 9 8.5S9.67 7 10.5 7s1.5.67 1.5 1.5-.67 1.5-1.5 1.5zm4.5 6c-.55 0-1-.45-1-1s.45-1 1-1 1 .45 1 1-.45 1-1 1z');
    svg.appendChild(path);
    reopen.appendChild(svg);
    document.body.appendChild(reopen);
  }

  function hideReopen() {
    if (reopen && reopen.parentNode) reopen.parentNode.removeChild(reopen);
    reopen = null;
  }

  // ---------------------------------------------------------------- start

  window.vuuutConsent = {
    open: openDialog,
    get: function () { return state ? { id: state.id, categories: list(state.granted).split(',') } : null; },
    allowed: function (cat) { return cat === 'necessary' || !!(state && state.granted[cat]); },
    on: function (fn) { if (typeof fn === 'function') listeners.push(fn); }
  };

  document.addEventListener('click', function (e) {
    var t = e.target && e.target.closest && e.target.closest('a[href="#cookie-settings"], [data-cookie-settings]');
    if (t) { e.preventDefault(); openDialog(); }
  });

  function start() {
    declarations();
    state = read();
    if (state) {
      apply();
      showReopen();
      return;
    }
    if (gpcOn()) {
      // The browser has already answered. Record it and do not ask.
      var g = all(cfg.mode !== 'opt_in');
      g.marketing = false;
      decide('gpc', g);
      return;
    }
    if (cfg.mode !== 'opt_in') {
      // Opt-out and notice: everything runs until refused. Nothing is stored
      // until the visitor answers, so the banner stays until they do.
      state = { id: uuid(), granted: all(true), action: 'pending', at: 0 };
      apply();
      state = null;
    } else {
      loadEmbeds();
    }
    showBanner();
  }

  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', start);
  else start();
})();
