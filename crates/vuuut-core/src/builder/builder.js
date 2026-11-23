/* Vuuut Page Builder — the site runtime.
 * Everything a layout does that CSS cannot: entrance animations, carousels,
 * tabs, counters, countdowns, rotating headlines, tables of contents, share
 * links, lightboxes, click-to-play videos, Lottie, off-canvas panels,
 * popups, mouse effects, reading progress, the posts filter and "load more",
 * and the nav menu's toggle. Each part binds once per element
 * (`data-vb-ready`), so the editor can call `vuuutBuilder.init(root)` after
 * every redraw. Without this script every layout still reads: animations are
 * only hidden once it has run (`.vb-js`), tabs fall back to stacked panels.
 */
(function () {
  'use strict';
  var reduce = window.matchMedia && matchMedia('(prefers-reduced-motion: reduce)').matches;
  document.documentElement.classList.add('vb-js');

  function all(root, sel) { return Array.prototype.slice.call(root.querySelectorAll(sel)); }
  function once(el, key) { var k = 'vbReady' + key; if (el.dataset[k]) return false; el.dataset[k] = '1'; return true; }
  function seen(el, cb, margin) {
    if (!('IntersectionObserver' in window)) { cb(); return; }
    var io = new IntersectionObserver(function (es) { es.forEach(function (e) { if (e.isIntersecting) { io.disconnect(); cb(); } }); }, {rootMargin: margin || '0px 0px -8% 0px'});
    io.observe(el);
  }
  function editing(el) { return !!el.closest('.vb-editing'); }

  // ---- entrance animations ----
  function anims(root) {
    all(root, '.vb-anim').forEach(function (el) {
      if (!once(el, 'Anim')) return;
      if (reduce || editing(el)) { el.classList.add('vb-seen'); return; }
      seen(el, function () { el.classList.add('vb-seen'); });
    });
  }

  // ---- carousels (images, slides, testimonials, posts) ----
  function carousels(root) {
    all(root, '[data-vb-carousel]').forEach(function (c) {
      if (!once(c, 'Car')) return;
      var track = c.querySelector('.vb-track');
      if (!track) return;
      var slides = all(track, ':scope > *');
      if (!slides.length) return;
      var fade = c.dataset.fade === '1';
      var idx = 0, timer = null;
      function perView() { return Math.max(1, Math.round(parseFloat(getComputedStyle(c).getPropertyValue('--vb-pv')) || 1)); }
      function pages() { return fade ? slides.length : Math.max(1, slides.length - perView() + 1); }
      function go(i, user) {
        var n = pages();
        if (i < 0) i = c.dataset.loop === 'true' ? n - 1 : 0;
        if (i >= n) i = c.dataset.loop === 'true' ? 0 : n - 1;
        idx = i;
        if (fade) slides.forEach(function (s, k) { s.classList.toggle('on', k === i); });
        else track.scrollTo({left: slides[i].offsetLeft - slides[0].offsetLeft, behavior: reduce ? 'auto' : 'smooth'});
        dots && all(dots, 'button').forEach(function (d, k) { d.setAttribute('aria-current', k === i ? 'true' : 'false'); });
        if (user) restart();
      }
      if (fade) { c.classList.add('vb-fade'); slides[0].classList.add('on'); }
      var dots = null;
      if (c.dataset.arrows !== 'false' && slides.length > 1) {
        var prev = document.createElement('button'), next = document.createElement('button');
        prev.type = next.type = 'button';
        prev.className = 'vb-car-arrow vb-prev'; next.className = 'vb-car-arrow vb-next';
        prev.setAttribute('aria-label', 'Previous'); next.setAttribute('aria-label', 'Next');
        prev.innerHTML = '<svg viewBox="0 0 24 24" fill="currentColor"><path d="M15.41 7.41 14 6l-6 6 6 6 1.41-1.41L10.83 12z"/></svg>';
        next.innerHTML = '<svg viewBox="0 0 24 24" fill="currentColor"><path d="M10 6 8.59 7.41 13.17 12l-4.58 4.59L10 18l6-6z"/></svg>';
        prev.onclick = function () { go(idx - 1, true); };
        next.onclick = function () { go(idx + 1, true); };
        c.appendChild(prev); c.appendChild(next);
      }
      if (c.dataset.dots !== 'false' && slides.length > 1) {
        dots = document.createElement('div');
        dots.className = 'vb-car-dots';
        for (var k = 0; k < pages(); k++) {
          (function (k) {
            var d = document.createElement('button');
            d.type = 'button'; d.setAttribute('aria-label', 'Slide ' + (k + 1));
            d.onclick = function () { go(k, true); };
            dots.appendChild(d);
          })(k);
        }
        c.appendChild(dots);
        go(0);
      }
      if (!fade) track.addEventListener('scroll', function () {
        var x = track.scrollLeft, best = 0, bd = Infinity;
        slides.forEach(function (s, k) { var d = Math.abs(s.offsetLeft - slides[0].offsetLeft - x); if (d < bd) { bd = d; best = k; } });
        if (best !== idx) { idx = best; dots && all(dots, 'button').forEach(function (d, k) { d.setAttribute('aria-current', k === idx ? 'true' : 'false'); }); }
      }, {passive: true});
      var ms = parseInt(c.dataset.autoplay, 10) || 0;
      function restart() { clearInterval(timer); if (ms && !reduce && !editing(c)) timer = setInterval(function () { go(idx + 1); }, ms); }
      c.addEventListener('mouseenter', function () { clearInterval(timer); });
      c.addEventListener('mouseleave', restart);
      c.addEventListener('focusin', function () { clearInterval(timer); });
      restart();
    });
  }

  // ---- tabs ----
  function tabs(root) {
    all(root, '[data-vb-tabs]').forEach(function (t) {
      if (!once(t, 'Tabs')) return;
      t.classList.add('vb-tabs-js');
      var btns = all(t, ':scope > .vb-tab-list > .vb-tab-btn');
      var panels = all(t, ':scope > .vb-tab-panels > .vb-tab-panel');
      function show(i) {
        btns.forEach(function (b, k) { b.classList.toggle('on', k === i); b.setAttribute('aria-selected', k === i ? 'true' : 'false'); b.tabIndex = k === i ? 0 : -1; });
        panels.forEach(function (p, k) { p.classList.toggle('on', k === i); });
      }
      btns.forEach(function (b, i) {
        b.addEventListener('click', function () { show(i); });
        b.addEventListener('keydown', function (e) {
          var d = e.key === 'ArrowRight' || e.key === 'ArrowDown' ? 1 : e.key === 'ArrowLeft' || e.key === 'ArrowUp' ? -1 : 0;
          if (!d) return;
          e.preventDefault();
          var j = (i + d + btns.length) % btns.length;
          show(j); btns[j].focus();
        });
      });
    });
  }

  // ---- counters and progress bars ----
  function counters(root) {
    all(root, '[data-vb-count]').forEach(function (el) {
      if (!once(el, 'Count')) return;
      var target = parseFloat(el.dataset.vbCount), dur = parseInt(el.dataset.duration, 10) || 1600;
      var dec = (el.dataset.vbCount.split('.')[1] || '').length;
      var fmt = function (v) { var s = v.toFixed(dec); return el.dataset.sep === 'true' ? Number(s).toLocaleString(undefined, {minimumFractionDigits: dec, maximumFractionDigits: dec}) : s; };
      if (reduce || editing(el)) { el.textContent = fmt(target); return; }
      el.textContent = fmt(0);
      seen(el, function () {
        var t0 = performance.now();
        (function step(now) {
          var p = Math.min(1, (now - t0) / dur), e = 1 - Math.pow(1 - p, 3);
          el.textContent = fmt(target * e);
          if (p < 1) requestAnimationFrame(step);
        })(t0);
      });
    });
    all(root, '.vb-progs').forEach(function (el) {
      if (!once(el, 'Prog')) return;
      if (reduce || editing(el)) { el.classList.add('vb-seen'); return; }
      seen(el, function () { el.classList.add('vb-seen'); });
    });
  }

  // ---- countdown ----
  function countdowns(root) {
    all(root, '[data-vb-countdown]').forEach(function (el) {
      if (!once(el, 'Cd')) return;
      var end;
      var ever = parseInt(el.dataset.evergreen, 10) || 0;
      if (ever) {
        var key = 'vb-cd-' + el.dataset.id, start;
        try { start = parseInt(localStorage.getItem(key), 10); if (!start) { start = Date.now(); localStorage.setItem(key, String(start)); } } catch (e) { start = Date.now(); }
        end = start + ever * 3600e3;
      } else {
        end = Date.parse((el.dataset.until || '') + (el.dataset.until && el.dataset.until.length <= 16 ? ':00Z' : ''));
      }
      var units = {};
      all(el, '[data-u]').forEach(function (u) { units[u.dataset.u] = u; });
      function pad(n) { return n < 10 ? '0' + n : String(n); }
      function tick() {
        if (isNaN(end)) return;
        var left = Math.max(0, end - Date.now());
        var s = Math.floor(left / 1000), d = Math.floor(s / 86400), hh = Math.floor(s % 86400 / 3600), mm = Math.floor(s % 3600 / 60), ss = s % 60;
        if (!units.d) hh += d * 24;
        if (units.d) units.d.textContent = pad(d);
        if (units.h) units.h.textContent = pad(hh);
        if (units.m) units.m.textContent = pad(mm);
        if (units.s) units.s.textContent = pad(ss);
        if (left <= 0 && !editing(el)) {
          clearInterval(timer);
          if (el.dataset.expiredHref) { location.href = el.dataset.expiredHref; return; }
          var done = el.querySelector('.vb-cd-done');
          if (done && done.textContent.trim()) { el.querySelector('.vb-cd-units').hidden = true; done.hidden = false; }
        }
      }
      var timer = setInterval(tick, 1000);
      tick();
    });
  }

  // ---- animated headline ----
  function headlines(root) {
    all(root, '[data-vb-rotate]').forEach(function (el) {
      if (!once(el, 'Rot')) return;
      var words = all(el, '.vb-aw'), i = 0, style = el.dataset.vbRotate, ms = parseInt(el.dataset.speed, 10) || 3000;
      if (words.length < 2 || reduce) return;
      if (style === 'typing') {
        var texts = words.map(function (w) { return w.textContent; });
        words.forEach(function (w, k) { if (k) w.remove(); });
        var w = words[0];
        w.classList.add('on', 'vb-typing');
        (function cycle() {
          var t = texts[i], n = t.length;
          (function erase() {
            if (n > 0) { n--; w.textContent = t.slice(0, n); setTimeout(erase, 40); return; }
            i = (i + 1) % texts.length; t = texts[i];
            (function type(k) { w.textContent = t.slice(0, k); if (k < t.length) setTimeout(function () { type(k + 1); }, 70); else setTimeout(cycle, ms); })(1);
          })();
        })();
        return;
      }
      setInterval(function () {
        var cur = words[i];
        i = (i + 1) % words.length;
        cur.classList.remove('on'); cur.classList.add('out');
        setTimeout(function () { cur.classList.remove('out'); }, 600);
        words[i].classList.add('on');
      }, ms);
    });
  }

  // ---- table of contents ----
  function slug(s) { return s.toLowerCase().replace(/[^\wÀ-￿]+/g, '-').replace(/^-|-$/g, '').slice(0, 60) || 'section'; }
  function tocs(root) {
    all(root, '[data-vb-toc]').forEach(function (t) {
      if (!once(t, 'Toc')) return;
      var scope = t.closest('article, main, .gh-content, .vb-page') || document.body;
      var nav = t.querySelector('.vb-toc-nav');
      var heads = all(scope, t.dataset.vbToc).filter(function (h) { return !t.contains(h) && !h.closest('.vb-oc, .vb-popup') && h.textContent.trim(); });
      if (!heads.length) { nav.innerHTML = '<p class="vb-toc-empty">Headings on this page appear here.</p>'; return; }
      var ol = document.createElement('ol'), used = {}, links = [];
      heads.forEach(function (h) {
        if (!h.id) { var base = slug(h.textContent), id = base, k = 2; while (used[id] || document.getElementById(id)) id = base + '-' + k++; h.id = id; }
        used[h.id] = 1;
        var li = document.createElement('li');
        li.className = 'vb-toc-' + h.tagName.toLowerCase();
        var a = document.createElement('a'); a.href = '#' + h.id; a.textContent = h.textContent.trim();
        li.appendChild(a); ol.appendChild(li); links.push([h, a]);
      });
      nav.appendChild(ol);
      if ('IntersectionObserver' in window) {
        var io = new IntersectionObserver(function (es) {
          es.forEach(function (e) { if (e.isIntersecting) links.forEach(function (p) { p[1].classList.toggle('on', p[0] === e.target); }); });
        }, {rootMargin: '0px 0px -70% 0px'});
        links.forEach(function (p) { io.observe(p[0]); });
      }
    });
  }

  // ---- share ----
  function shares(root) {
    all(root, '[data-vb-share]').forEach(function (a) {
      if (!once(a, 'Share')) return;
      var u = encodeURIComponent(location.href.split('#')[0]), t = encodeURIComponent(document.title);
      var map = {
        x: 'https://x.com/intent/post?url=' + u + '&text=' + t,
        facebook: 'https://www.facebook.com/sharer/sharer.php?u=' + u,
        linkedin: 'https://www.linkedin.com/sharing/share-offsite/?url=' + u,
        reddit: 'https://www.reddit.com/submit?url=' + u + '&title=' + t,
        whatsapp: 'https://api.whatsapp.com/send?text=' + t + '%20' + u,
        telegram: 'https://t.me/share/url?url=' + u + '&text=' + t,
        pinterest: 'https://pinterest.com/pin/create/button/?url=' + u + '&description=' + t,
        bluesky: 'https://bsky.app/intent/compose?text=' + t + '%20' + u,
        threads: 'https://www.threads.net/intent/post?text=' + t + '%20' + u,
        email: 'mailto:?subject=' + t + '&body=' + u
      };
      var net = a.dataset.vbShare;
      if (net === 'copy') {
        a.addEventListener('click', function (e) {
          e.preventDefault();
          var lab = a.querySelector('span'), was = lab && lab.textContent;
          (navigator.clipboard ? navigator.clipboard.writeText(location.href) : Promise.reject()).then(function () {
            if (lab) { lab.textContent = 'Copied'; setTimeout(function () { lab.textContent = was; }, 1600); }
          }).catch(function () { window.prompt('Copy this link', location.href); });
        });
        return;
      }
      if (map[net]) { a.href = map[net]; if (net !== 'email') a.target = '_blank'; }
    });
  }

  // ---- lightbox ----
  var lb = null;
  function lightbox(root) {
    all(root, '[data-vb-lightbox]').forEach(function (a) {
      if (!once(a, 'Lb')) return;
      a.addEventListener('click', function (e) {
        if (editing(a)) return;
        e.preventDefault();
        var group = all(document, '[data-vb-lightbox="' + a.dataset.vbLightbox + '"]'), i = group.indexOf(a);
        if (!lb) {
          lb = document.createElement('div');
          lb.className = 'vb-lightbox';
          lb.innerHTML = '<button type="button" class="vb-lb-x" aria-label="Close">×</button><button type="button" class="vb-lb-prev" aria-label="Previous">‹</button><figure><img alt=""><figcaption></figcaption></figure><button type="button" class="vb-lb-next" aria-label="Next">›</button>';
          lb.setAttribute('role', 'dialog'); lb.setAttribute('aria-modal', 'true');
          document.body.appendChild(lb);
        }
        function show(k) {
          i = (k + group.length) % group.length;
          lb.querySelector('img').src = group[i].getAttribute('href');
          lb.querySelector('figcaption').textContent = group[i].dataset.caption || '';
          lb.querySelector('.vb-lb-prev').hidden = lb.querySelector('.vb-lb-next').hidden = group.length < 2;
        }
        function close() { lb.classList.remove('on'); document.removeEventListener('keydown', key); a.focus(); }
        function key(e) { if (e.key === 'Escape') close(); if (e.key === 'ArrowRight') show(i + 1); if (e.key === 'ArrowLeft') show(i - 1); }
        lb.querySelector('.vb-lb-x').onclick = close;
        lb.querySelector('.vb-lb-prev').onclick = function () { show(i - 1); };
        lb.querySelector('.vb-lb-next').onclick = function () { show(i + 1); };
        lb.onclick = function (e) { if (e.target === lb) close(); };
        document.addEventListener('keydown', key);
        show(i);
        lb.classList.add('on');
        lb.querySelector('.vb-lb-x').focus();
      });
    });
  }

  // ---- click-to-play videos and playlists ----
  function videos(root) {
    all(root, '[data-vb-video-dialog]').forEach(function (trigger) {
      if (!once(trigger, 'VideoDialog') || typeof HTMLDialogElement === 'undefined') return;
      trigger.addEventListener('click', function (e) {
        var url;
        try { url = new URL(trigger.dataset.vbVideoDialog); } catch (_) { return; }
        if (url.origin !== 'https://www.youtube-nocookie.com' && url.origin !== 'https://player.vimeo.com') return;
        e.preventDefault();
        var dialog = document.createElement('dialog');
        dialog.className = 'vb-video-dialog';
        var title = trigger.textContent.trim() || 'Video';
        dialog.setAttribute('aria-label', title);
        var close = document.createElement('button');
        close.type = 'button';
        close.setAttribute('aria-label', 'Close video');
        close.textContent = '×';
        close.onclick = function () { dialog.close(); };
        var frame = document.createElement('iframe');
        frame.src = url.href;
        frame.allow = 'autoplay; encrypted-media; picture-in-picture';
        frame.allowFullscreen = true;
        frame.title = title;
        dialog.appendChild(close);
        dialog.appendChild(frame);
        dialog.addEventListener('click', function (event) {
          if (event.target === dialog) {
            var bounds = dialog.getBoundingClientRect();
            if (event.clientX < bounds.left || event.clientX > bounds.right ||
                event.clientY < bounds.top || event.clientY > bounds.bottom) dialog.close();
          }
        });
        dialog.addEventListener('close', function () {
          dialog.remove();
          trigger.focus();
        }, {once:true});
        document.body.appendChild(dialog);
        dialog.showModal();
        close.focus();
      });
    });
    all(root, '.vb-facade').forEach(function (b) {
      if (!once(b, 'Fac')) return;
      b.addEventListener('click', function () {
        if (editing(b)) return;
        var f = document.createElement('iframe');
        f.src = b.dataset.vbSrc.replace(/&amp;/g, '&');
        f.allow = 'accelerometer; autoplay; encrypted-media; gyroscope; picture-in-picture';
        f.allowFullscreen = true; f.title = 'Video';
        b.replaceWith(f);
      });
    });
    all(root, '.vb-playlist').forEach(function (p) {
      if (!once(p, 'Pl')) return;
      all(p, '.vb-pl-item').forEach(function (it) {
        it.addEventListener('click', function () {
          all(p, '.vb-pl-item').forEach(function (x) { x.classList.toggle('on', x === it); });
          var player = p.querySelector('.vb-pl-player'), src = it.dataset.vbSrc.replace(/&amp;/g, '&');
          if (/youtube|vimeo/.test(src)) player.innerHTML = '<iframe allow="autoplay; encrypted-media; picture-in-picture" allowfullscreen title="Video"></iframe>';
          else player.innerHTML = '<video controls autoplay playsinline></video>';
          player.firstChild.src = src;
        });
      });
    });
  }

  // ---- lottie ----
  var lottieLoading = null;
  function lotties(root) {
    var els = all(root, '[data-vb-lottie]').filter(function (el) { return once(el, 'Lot'); });
    if (!els.length) return;
    if (!lottieLoading) lottieLoading = new Promise(function (res, rej) {
      if (window.lottie) { res(); return; }
      var s = document.createElement('script');
      s.src = 'https://cdnjs.cloudflare.com/ajax/libs/lottie-web/5.12.2/lottie_light.min.js';
      s.onload = res; s.onerror = rej;
      document.head.appendChild(s);
    });
    lottieLoading.then(function () {
      els.forEach(function (el) {
        var trig = el.dataset.trigger;
        var a = window.lottie.loadAnimation({container: el, renderer: 'svg', loop: el.dataset.loop === 'true', autoplay: trig === 'always', path: el.dataset.vbLottie});
        a.setSpeed(parseFloat(el.dataset.speed) || 1);
        if (trig === 'view') seen(el, function () { a.play(); });
        if (trig === 'hover') { el.addEventListener('mouseenter', function () { a.play(); }); el.addEventListener('mouseleave', function () { a.pause(); }); }
        if (trig === 'scroll') a.addEventListener('DOMLoaded', function () {
          var onScroll = function () { var r = el.getBoundingClientRect(), p = 1 - Math.min(1, Math.max(0, (r.top + r.height) / (innerHeight + r.height))); a.goToAndStop(p * a.totalFrames, true); };
          addEventListener('scroll', onScroll, {passive: true}); onScroll();
        });
      });
    }).catch(function () { els.forEach(function (el) { el.textContent = ''; }); });
  }

  // ---- off-canvas, alerts, code, hotspots ----
  function panels(root) {
    all(root, '.vb-oc-open').forEach(function (b) {
      if (!once(b, 'Oc')) return;
      var oc = document.getElementById(b.getAttribute('aria-controls'));
      if (!oc) return;
      function close() { oc.classList.remove('on'); b.setAttribute('aria-expanded', 'false'); setTimeout(function () { oc.hidden = true; }, 300); b.focus(); }
      b.addEventListener('click', function () {
        if (editing(b)) return;
        oc.hidden = false; requestAnimationFrame(function () { oc.classList.add('on'); });
        b.setAttribute('aria-expanded', 'true');
        var x = oc.querySelector('.vb-oc-x'); x && x.focus();
      });
      oc.addEventListener('click', function (e) { if (e.target === oc || e.target.closest('.vb-oc-x')) close(); });
      document.addEventListener('keydown', function (e) { if (e.key === 'Escape' && oc.classList.contains('on')) close(); });
    });
    all(root, '.vb-alert-x').forEach(function (x) { if (once(x, 'Al')) x.addEventListener('click', function () { x.closest('.vb-alert').remove(); }); });
    all(root, '.vb-copy').forEach(function (b) {
      if (!once(b, 'Copy')) return;
      b.addEventListener('click', function () {
        var code = b.closest('.vb-code').querySelector('code').innerText;
        navigator.clipboard && navigator.clipboard.writeText(code).then(function () { b.textContent = 'Copied'; setTimeout(function () { b.textContent = 'Copy'; }, 1500); });
      });
    });
    all(root, '.vb-hs-click .vb-spot-dot').forEach(function (d) {
      if (!once(d, 'Hs')) return;
      d.addEventListener('click', function () { var s = d.closest('.vb-spot'); all(s.parentNode, '.vb-spot').forEach(function (o) { if (o !== s) o.classList.remove('open'); }); s.classList.toggle('open'); });
    });
  }

  // ---- mouse effects ----
  function mouse(root) {
    if (reduce || matchMedia('(hover: none)').matches) return;
    all(root, '[data-vb-mouse]').forEach(function (el) {
      if (!once(el, 'Mouse') || editing(el)) return;
      var s = parseInt(el.dataset.vbMs, 10) || 4, kind = el.dataset.vbMouse;
      var host = kind === 'track' ? (el.closest('.vb-s') || el) : el;
      host.addEventListener('mousemove', function (e) {
        var r = host.getBoundingClientRect(), x = (e.clientX - r.left) / r.width - .5, y = (e.clientY - r.top) / r.height - .5;
        el.style.transform = kind === 'tilt' ? 'perspective(800px) rotateX(' + (-y * s * 3) + 'deg) rotateY(' + (x * s * 3) + 'deg)' : 'translate(' + (x * s * 6) + 'px,' + (y * s * 6) + 'px)';
      });
      host.addEventListener('mouseleave', function () { el.style.transform = ''; });
    });
  }

  // ---- reading progress ----
  function progress(root) {
    var els = all(root, '[data-vb-progress]').filter(function (el) { return once(el, 'Pt'); });
    if (!els.length) return;
    function upd() {
      var h = document.documentElement, max = h.scrollHeight - innerHeight, p = max > 0 ? Math.min(100, Math.max(0, h.scrollTop / max * 100)) : 0;
      els.forEach(function (el) { el.style.setProperty('--vb-p', p.toFixed(1)); var b = el.querySelector('b'); if (b) b.textContent = Math.round(p) + '%'; });
    }
    addEventListener('scroll', upd, {passive: true}); upd();
  }

  // ---- posts: tag filter and load more ----
  function posts(root) {
    all(root, '.vb-posts-wrap').forEach(function (w) {
      if (!once(w, 'Posts')) return;
      var cards = all(w, '.vb-pc'), more = w.querySelector('.vb-more'), shown = parseInt(w.dataset.count, 10) || cards.length, filter = '*';
      function apply() {
        var n = 0;
        cards.forEach(function (c) {
          var match = filter === '*' || (' ' + c.dataset.tags + ' ').indexOf(' ' + filter + ' ') >= 0;
          var vis = match && n < shown;
          if (match) n++;
          c.hidden = !vis;
        });
        if (more) more.hidden = n <= shown;
      }
      all(w, '.vb-filter button').forEach(function (b) {
        b.addEventListener('click', function () { filter = b.dataset.tag; all(w, '.vb-filter button').forEach(function (x) { x.classList.toggle('on', x === b); }); apply(); });
      });
      if (more) more.addEventListener('click', function () { shown += parseInt(w.dataset.count, 10) || 6; apply(); });
      apply();
    });
  }

  // ---- nav menu toggle ----
  function menus(root) {
    all(root, '.vb-menu-toggle').forEach(function (b) {
      if (!once(b, 'Menu')) return;
      b.addEventListener('click', function () {
        var m = b.closest('.vb-menu'), open = !m.classList.contains('open');
        m.classList.toggle('open', open);
        b.setAttribute('aria-expanded', open ? 'true' : 'false');
      });
    });
  }

  // ---- popups (Theme Builder) ----
  function popups(root) {
    all(root, '.vb-popup').forEach(function (p) {
      if (!once(p, 'Pop')) return;
      var id = p.dataset.id, freq = p.dataset.freq || 'session', key = 'vb-pop-' + id, store = freq === 'session' ? sessionStorage : localStorage;
      function allowed() {
        if (freq === 'always') return true;
        try {
          var last = parseInt(store.getItem(key), 10);
          if (!last) return true;
          if (freq === 'once' || freq === 'session') return false;
          var gap = freq === 'day' ? 864e5 : 6048e5;
          return Date.now() - last > gap;
        } catch (e) { return true; }
      }
      function open(force) {
        if (p.classList.contains('on') || (!force && !allowed())) return;
        try { store.setItem(key, String(Date.now())); } catch (e) { /* private mode */ }
        p.hidden = false;
        requestAnimationFrame(function () { p.classList.add('on'); });
        var x = p.querySelector('.vb-pop-x'); x && x.focus();
      }
      function close() { p.classList.remove('on'); setTimeout(function () { p.hidden = true; }, 300); }
      p.addEventListener('click', function (e) { if ((e.target === p && p.dataset.overlayClose !== 'false') || e.target.closest('.vb-pop-x')) close(); });
      document.addEventListener('keydown', function (e) { if (e.key === 'Escape' && p.classList.contains('on')) close(); });
      // a link to #popup-<id> opens it from anywhere
      document.addEventListener('click', function (e) {
        var a = e.target.closest && e.target.closest('a[href="#popup-' + id + '"]');
        if (a) { e.preventDefault(); open(true); }
      });
      var trig = p.dataset.trigger, val = parseFloat(p.dataset.value) || 0;
      if (trig === 'load') setTimeout(open, val * 1000);
      if (trig === 'scroll') {
        var onScroll = function () {
          var h = document.documentElement, pct = h.scrollTop / Math.max(1, h.scrollHeight - innerHeight) * 100;
          if (pct >= val) { removeEventListener('scroll', onScroll); open(); }
        };
        addEventListener('scroll', onScroll, {passive: true});
      }
      if (trig === 'exit') document.addEventListener('mouseout', function onOut(e) { if (!e.relatedTarget && e.clientY <= 0) { document.removeEventListener('mouseout', onOut); open(); } });
      if (trig === 'idle') {
        var t; var reset = function () { clearTimeout(t); t = setTimeout(open, (val || 30) * 1000); };
        ['mousemove', 'keydown', 'scroll', 'touchstart'].forEach(function (ev) { addEventListener(ev, reset, {passive: true}); });
        reset();
      }
    });
  }

  function init(root) {
    root = root || document;
    [anims, carousels, tabs, counters, countdowns, headlines, tocs, shares, lightbox, videos, lotties, panels, mouse, progress, posts, menus, popups].forEach(function (f) {
      try { f(root); } catch (e) { if (window.console) console.error('vuuut builder:', e); }
    });
  }
  window.vuuutBuilder = {init: init};
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', function () { init(document); });
  else init(document);
})();
