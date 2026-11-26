/* The podcast player: one dock per page, driven by data attributes.
 *
 *   data-vp-play data-id data-src data-title data-show data-art data-url data-dur
 *       — an element that plays an episode when clicked
 *   data-vp-stage (+ the same attributes) with data-vp-toggle and data-vp-bar
 *       — the big player on an episode page
 *   data-vp-seek="123"   — jump to 2:03 in the page's episode
 *   data-vp-copy="text"  — copy to the clipboard
 *   data-vp-share        — copy this page's address at the current time
 *   data-vp-filter       — an input that filters the transcript's cues
 *
 * Everything works without it: the episode page has a native <audio>, and
 * every list links to the episode. Text from the page is placed with
 * textContent only. Positions are kept in localStorage per episode, and the
 * episode playing is carried to the next page in sessionStorage (paused —
 * browsers do not allow playing on arrival).
 */
(function () {
  'use strict';
  if (window.__vuuutPodcast) { return; }
  window.__vuuutPodcast = true;

  var SPEEDS = [1, 1.25, 1.5, 1.75, 2, 0.75];
  var ICON = {
    play: 'M8 5.14v13.72a1 1 0 0 0 1.5.86l11-6.86a1 1 0 0 0 0-1.72l-11-6.86A1 1 0 0 0 8 5.14z',
    pause: 'M7 5h3.5v14H7zM13.5 5H17v14h-3.5z',
    close: 'M18.3 5.71 12 12l6.3 6.29-1.41 1.42L10.59 13.4 4.29 19.7 2.88 18.3 9.17 12 2.88 5.71 4.29 4.29l6.3 6.3 6.29-6.3z'
  };
  var audio = new Audio();
  audio.preload = 'none';
  var current = null;
  var dock = null, els = {};
  var pendingStart = null;
  var lastSave = 0;

  function store(k, v) { try { if (v === null) { localStorage.removeItem(k); } else { localStorage.setItem(k, v); } } catch (e) {} }
  function read(k) { try { return localStorage.getItem(k); } catch (e) { return null; } }
  function session(k, v) { try { if (v === undefined) { return sessionStorage.getItem(k); } if (v === null) { sessionStorage.removeItem(k); } else { sessionStorage.setItem(k, v); } } catch (e) { return null; } }
  function clock(s) {
    s = Math.max(0, Math.floor(s || 0));
    var h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), x = s % 60;
    return (h ? h + ':' + (m < 10 ? '0' : '') : '') + m + ':' + (x < 10 ? '0' : '') + x;
  }
  function parseClock(t) {
    if (!t) { return null; }
    var parts = String(t).split(':').map(Number);
    if (parts.some(isNaN)) { return null; }
    return parts.reduce(function (a, b) { return a * 60 + b; }, 0);
  }
  function svg(path) {
    var ns = 'http://www.w3.org/2000/svg';
    var s = document.createElementNS(ns, 'svg'); s.setAttribute('viewBox', '0 0 24 24'); s.setAttribute('aria-hidden', 'true');
    var p = document.createElementNS(ns, 'path'); p.setAttribute('d', path); s.appendChild(p);
    return s;
  }
  function button(label, content, attr) {
    var b = document.createElement('button'); b.type = 'button'; b.setAttribute('aria-label', label); b.title = label;
    if (attr) { b.setAttribute(attr, ''); }
    if (typeof content === 'string') { b.textContent = content; } else { b.appendChild(content); }
    return b;
  }

  function buildDock() {
    if (dock) { return; }
    dock = document.createElement('div');
    dock.className = 'vp-dock';
    dock.setAttribute('role', 'region');
    dock.setAttribute('aria-label', 'Podcast player');
    var stage = document.querySelector('.vp');
    if (stage) { var cs = getComputedStyle(stage); dock.style.setProperty('--vp-accent', cs.getPropertyValue('--vp-accent')); }
    els.art = document.createElement('img'); els.art.alt = '';
    els.body = document.createElement('div'); els.body.className = 'vp-dock-body';
    els.title = document.createElement('a'); els.title.className = 'vp-dock-title';
    els.show = document.createElement('span'); els.show.className = 'vp-dock-show';
    var bar = document.createElement('div'); bar.className = 'vp-dock-bar';
    els.now = document.createElement('span'); els.now.textContent = '0:00';
    els.range = document.createElement('input'); els.range.type = 'range'; els.range.min = 0; els.range.max = 1000; els.range.value = 0; els.range.setAttribute('aria-label', 'Position');
    els.left = document.createElement('span'); els.left.textContent = '';
    bar.appendChild(els.now); bar.appendChild(els.range); bar.appendChild(els.left);
    els.body.appendChild(els.title); els.body.appendChild(els.show); els.body.appendChild(bar);
    var controls = document.createElement('div'); controls.className = 'vp-dock-controls';
    els.back = button('Back 15 seconds', '−15', 'data-vp-back');
    els.toggle = button('Play', svg(ICON.play)); els.toggle.className = 'vp-dock-toggle';
    els.fwd = button('Forward 30 seconds', '+30');
    els.speed = button('Playback speed', '1×', 'data-vp-speed');
    els.close = button('Close player', svg(ICON.close));
    [els.back, els.toggle, els.fwd, els.speed, els.close].forEach(function (b) { controls.appendChild(b); });
    dock.appendChild(els.art); dock.appendChild(els.body); dock.appendChild(controls);
    document.body.appendChild(dock);

    els.toggle.addEventListener('click', toggle);
    els.back.addEventListener('click', function () { audio.currentTime = Math.max(0, audio.currentTime - 15); });
    els.fwd.addEventListener('click', function () { audio.currentTime = Math.min(audio.duration || 1e9, audio.currentTime + 30); });
    els.speed.addEventListener('click', function () {
      var i = (SPEEDS.indexOf(audio.playbackRate) + 1) % SPEEDS.length;
      setSpeed(SPEEDS[i]);
    });
    els.close.addEventListener('click', function () { audio.pause(); dock.classList.remove('is-open'); session('vp:now', null); });
    els.range.addEventListener('input', function () {
      if (audio.duration) { audio.currentTime = audio.duration * (els.range.value / 1000); }
    });
  }

  function setSpeed(r) {
    audio.playbackRate = r;
    store('vp:speed', String(r));
    if (els.speed) { els.speed.textContent = (r === 1 ? '1' : r) + '×'; }
  }

  function load(d, autoplay) {
    buildDock();
    current = { id: d.id, src: d.src, title: d.title || '', show: d.show || '', art: d.art || '', url: d.url || '', dur: +d.dur || 0 };
    audio.src = current.src;
    els.title.textContent = current.title;
    els.title.href = current.url || '#';
    els.show.textContent = current.show;
    if (current.art) { els.art.src = current.art; els.art.hidden = false; } else { els.art.removeAttribute('src'); }
    els.left.textContent = current.dur ? clock(current.dur) : '';
    dock.classList.add('is-open');
    var saved = parseFloat(read('vp:pos:' + current.id) || '0');
    var start = pendingStart !== null ? pendingStart : (saved && (!current.dur || saved < current.dur - 20) ? saved : 0);
    pendingStart = null;
    var seekOnce = function () { if (start) { audio.currentTime = start; } audio.removeEventListener('loadedmetadata', seekOnce); };
    audio.addEventListener('loadedmetadata', seekOnce);
    setSpeed(parseFloat(read('vp:speed') || '1') || 1);
    if ('mediaSession' in navigator && window.MediaMetadata) {
      try {
        navigator.mediaSession.metadata = new MediaMetadata({ title: current.title, artist: current.show, artwork: current.art ? [{ src: current.art, sizes: '512x512' }] : [] });
        navigator.mediaSession.setActionHandler('seekbackward', function () { audio.currentTime = Math.max(0, audio.currentTime - 15); });
        navigator.mediaSession.setActionHandler('seekforward', function () { audio.currentTime += 30; });
      } catch (e) {}
    }
    session('vp:now', JSON.stringify(current));
    if (autoplay) { audio.play().catch(function () {}); } else { audio.load(); }
    paint();
  }

  function start(d) {
    if (current && current.id === d.id) { toggle(); return; }
    load(d, true);
  }

  function toggle() {
    if (!current) { return; }
    if (audio.paused) { audio.play().catch(function () {}); } else { audio.pause(); }
  }

  function save(force) {
    if (!current || !audio.currentTime) { return; }
    var now = Date.now();
    if (!force && now - lastSave < 4000) { return; }
    lastSave = now;
    var nearEnd = audio.duration && audio.currentTime > audio.duration - 20;
    store('vp:pos:' + current.id, nearEnd ? null : String(Math.floor(audio.currentTime)));
  }

  function paint() {
    var playing = current && !audio.paused;
    if (els.toggle) {
      els.toggle.textContent = '';
      els.toggle.appendChild(svg(playing ? ICON.pause : ICON.play));
      els.toggle.setAttribute('aria-label', playing ? 'Pause' : 'Play');
    }
    document.querySelectorAll('[data-vp-play]').forEach(function (b) {
      var on = playing && current && b.getAttribute('data-id') === current.id;
      b.classList.toggle('is-playing', !!on);
      var t = b.hasAttribute('data-vp-stage') ? b.querySelector('[data-vp-toggle]') : null;
      if (t) { t.textContent = ''; t.appendChild(svg(on ? ICON.pause : ICON.play)); t.setAttribute('aria-label', on ? 'Pause' : 'Play'); }
    });
  }

  function tick() {
    if (!current) { return; }
    var d = audio.duration || current.dur || 0;
    els.now.textContent = clock(audio.currentTime);
    els.left.textContent = d ? '-' + clock(d - audio.currentTime) : '';
    if (d && document.activeElement !== els.range) { els.range.value = Math.round(audio.currentTime / d * 1000); }
    document.querySelectorAll('[data-vp-stage]').forEach(function (st) {
      if (st.getAttribute('data-id') !== current.id) { return; }
      var fill = st.querySelector('[data-vp-bar] span');
      if (fill && d) { fill.style.width = (audio.currentTime / d * 100) + '%'; }
    });
    // the chapter being played
    var items = document.querySelectorAll('.vp-chapters li');
    var cur = null;
    items.forEach(function (li) { var b = li.querySelector('[data-vp-seek]'); if (b && +b.getAttribute('data-vp-seek') <= audio.currentTime) { cur = li; } });
    items.forEach(function (li) { li.classList.toggle('is-current', li === cur); });
    save(false);
  }

  audio.addEventListener('play', paint);
  audio.addEventListener('pause', function () { paint(); save(true); });
  audio.addEventListener('timeupdate', tick);
  audio.addEventListener('ended', function () { if (current) { store('vp:pos:' + current.id, null); } paint(); });
  window.addEventListener('pagehide', function () { save(true); });

  function pageEpisode() {
    var st = document.querySelector('[data-vp-stage]');
    return st ? st.dataset : null;
  }

  function seek(n) {
    var d = pageEpisode();
    if (d && (!current || current.id !== d.id)) { pendingStart = n; load(d, true); return; }
    if (!current) { return; }
    audio.currentTime = n;
    if (audio.paused) { audio.play().catch(function () {}); }
  }

  function copy(text, el) {
    var done = function () { if (!el) { return; } var was = el.textContent; el.textContent = 'Copied'; setTimeout(function () { el.textContent = was; }, 1600); };
    if (navigator.clipboard && navigator.clipboard.writeText) { navigator.clipboard.writeText(text).then(done, function () {}); }
    else { var t = document.createElement('textarea'); t.value = text; document.body.appendChild(t); t.select(); try { document.execCommand('copy'); done(); } catch (e) {} t.remove(); }
  }

  document.addEventListener('click', function (e) {
    var t = e.target;
    if (!t.closest) { return; }
    var tg = t.closest('[data-vp-toggle]');
    if (tg) { var st = tg.closest('[data-vp-stage]'); if (st) { e.preventDefault(); start(st.dataset); return; } }
    var bar = t.closest('[data-vp-bar]');
    if (bar) {
      var s2 = bar.closest('[data-vp-stage]');
      var r = bar.getBoundingClientRect();
      var f = Math.min(1, Math.max(0, (e.clientX - r.left) / r.width));
      var dur = (current && s2 && current.id === s2.dataset.id && audio.duration) || +s2.dataset.dur || 0;
      if (dur) { seek(f * dur); }
      return;
    }
    var p = t.closest('[data-vp-play]');
    if (p && !p.hasAttribute('data-vp-stage')) { e.preventDefault(); start(p.dataset); return; }
    var sk = t.closest('[data-vp-seek]');
    if (sk) { e.preventDefault(); seek(+sk.getAttribute('data-vp-seek') || 0); return; }
    var cp = t.closest('[data-vp-copy]');
    if (cp) { e.preventDefault(); copy(cp.getAttribute('data-vp-copy'), cp.tagName === 'BUTTON' ? cp : null); return; }
    var sh = t.closest('[data-vp-share]');
    if (sh) {
      var at = current ? Math.floor(audio.currentTime) : 0;
      copy(location.href.split('#')[0] + (at ? '#t=' + clock(at) : ''), sh);
      return;
    }
    var ot = t.closest('[data-vp-open-transcript]');
    if (ot) { var d2 = document.getElementById('transcript'); if (d2) { d2.open = true; } }
  });

  document.addEventListener('input', function (e) {
    var f = e.target.closest && e.target.closest('[data-vp-filter]');
    if (!f) { return; }
    var q = f.value.trim().toLowerCase();
    var box = f.closest('.vp-transcript') || document;
    box.querySelectorAll('.vp-cue').forEach(function (c) { c.hidden = !!q && c.textContent.toLowerCase().indexOf(q) < 0; });
  });


  // Some themes put page content in a column that is not centred on the
  // page; the wide layout centres on the column, so nudge it back to the
  // middle of the viewport.
  function centre(sel) {
    function run() {
      document.querySelectorAll(sel).forEach(function (el) {
        el.style.marginLeft = '';
        var r = el.getBoundingClientRect();
        var off = (document.documentElement.clientWidth - r.width) / 2 - r.left;
        if (Math.abs(off) > 1) { el.style.marginLeft = off + 'px'; }
      });
    }
    run();
    window.addEventListener('resize', run);
  }
  function ready() {
    document.querySelectorAll('.vp').forEach(function (el) { el.classList.add('vp-js'); });
    centre('.vp');
    // #t=12:30 on an episode page starts there
    var m = /[#&]t=([0-9:]+)/.exec(location.hash);
    if (m && pageEpisode()) { pendingStart = parseClock(m[1]); }
    // what was playing on the last page, paused where it was
    var prev = null;
    try { prev = JSON.parse(session('vp:now') || 'null'); } catch (e) { prev = null; }
    if (prev && prev.id && prev.src) {
      if (pendingStart !== null && pageEpisode() && pageEpisode().id !== prev.id) { return; }
      load(prev, false);
    }
  }
  if (document.readyState === 'loading') { document.addEventListener('DOMContentLoaded', ready); } else { ready(); }
})();
