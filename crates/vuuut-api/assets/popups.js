/* Popups & lead generation — the visitor's half.
 *
 * Loaded on a page only when at least one popup is live (see
 * `vuuut_api::popups::inject`). It asks the server what this reader may see
 * on this path, waits for the trigger, draws the popup and posts the address.
 *
 * The server is the authority on everything that matters. Which popups come
 * back is decided there from the path, the member cookie and the schedule;
 * this file decides only *when* to show what it was given, and every value it
 * places in the page goes in through `textContent`, never `innerHTML`.
 *
 * No framework, no build step, ES5-era syntax in an IIFE.
 */
(function () {
  'use strict';

  var script = document.currentScript;
  var base = (script && script.getAttribute('data-popups')) || '';
  var STORE = 'ghost:popup:';
  var shown = false; // one popup per page view, whatever else matches
  /* Settings → Popups, as the server handed them over with the list. */
  var site = { hide_on_mobile: false, cooldown_hours: 0, consent: '' };
  var LAST = 'last'; // the site-wide "any popup was shown" timestamp

  function store(key) {
    try {
      return window.localStorage.getItem(STORE + key);
    } catch (e) {
      // Private mode, or storage switched off. A reader who cannot be
      // remembered sees it once per page view rather than never.
      return null;
    }
  }
  function remember(key, value) {
    try {
      window.localStorage.setItem(STORE + key, value);
    } catch (e) {}
  }

  /* Whether this browser has already had its turn. "once" is forever,
   * "days" waits, "always" never suppresses. A closed popup counts: somebody
   * who dismissed it has answered. */
  function due(p) {
    /* Spacing between ANY two popups, for this visitor, across the site.
     * Checked before the popup's own frequency, which still applies after. */
    if (site.cooldown_hours > 0) {
      var any = parseInt(store(LAST) || '0', 10);
      if (any && Date.now() - any < site.cooldown_hours * 3600000) return false;
    }
    if (p.frequency === 'always') return true;
    var last = parseInt(store(p.id) || '0', 10);
    if (!last) return true;
    if (p.frequency === 'once') return false;
    var days = p.frequency_days > 0 ? p.frequency_days : 30;
    return Date.now() - last > days * 86400000;
  }

  function el(tag, cls, text) {
    var node = document.createElement(tag);
    if (cls) node.className = cls;
    if (text) node.textContent = text;
    return node;
  }

  function beacon(path, body) {
    try {
      if (navigator.sendBeacon) {
        navigator.sendBeacon(base + path, body || new Blob());
        return;
      }
    } catch (e) {}
    fetch(base + path, { method: 'POST', credentials: 'same-origin', body: body || '' }).catch(function () {});
  }

  /* The stylesheet is fetched the first time a popup is actually drawn, not
   * when the script loads: most page views arm a trigger that never fires,
   * and those should cost nothing beyond the one JSON request. */
  var styled = false;
  function styleOnce() {
    if (styled) return;
    styled = true;
    var link = document.createElement('link');
    link.rel = 'stylesheet';
    link.href = (script && script.src ? script.src.replace(/popups\.js.*$/, 'popups.css') : base + '/popups/api/assets/0/popups.css');
    document.head.appendChild(link);
  }

  /* Whether the reader is already in a dialog of the site's own — Portal's
   * signup or account sheet, or the search panel. A popup opening on top of
   * one is two modals stacked, and the reader is in the middle of something;
   * it waits until they are done instead. */
  function busy() {
    return !!document.querySelector('#ghost-portal-root iframe[title="portal-popup"], #sodo-search-root iframe');
  }

  function render(p) {
    if (shown) return;
    if (busy()) {
      setTimeout(function () { render(p); }, 1500);
      return;
    }
    shown = true;
    styleOnce();
    remember(p.id, String(Date.now()));
    remember(LAST, String(Date.now()));
    beacon('/popups/api/' + p.id + '/seen/');

    var root = el('div', 'gh-popup gh-popup-' + (p.kind || 'modal').replace('_', '-'));
    root.setAttribute('role', 'dialog');
    root.setAttribute('aria-modal', p.kind === 'modal' ? 'true' : 'false');
    root.setAttribute('aria-label', p.heading || 'Message');

    var box = el('div', 'gh-popup-box');
    var close = el('button', 'gh-popup-close', '×');
    close.type = 'button';
    close.setAttribute('aria-label', 'Close');
    box.appendChild(close);

    if (p.heading) box.appendChild(el('h2', 'gh-popup-heading', p.heading));
    if (p.body) box.appendChild(el('p', 'gh-popup-body', p.body));

    var done = el('p', 'gh-popup-done');
    done.hidden = true;
    box.appendChild(done);

    if (p.capture_email) {
      var form = el('form', 'gh-popup-form');
      form.noValidate = true;

      if (p.capture_name) {
        var nameInput = el('input', 'gh-popup-input');
        nameInput.type = 'text';
        nameInput.name = 'name';
        nameInput.placeholder = 'Your name';
        nameInput.autocomplete = 'name';
        form.appendChild(nameInput);
      }

      var email = el('input', 'gh-popup-input');
      email.type = 'email';
      email.name = 'email';
      email.required = true;
      email.placeholder = 'you@example.com';
      email.autocomplete = 'email';
      form.appendChild(email);

      /* The honeypot. Hidden from people by CSS and from screen readers by
       * aria-hidden and tabindex, so only something filling in every field
       * fills it in. */
      var trap = el('input', 'gh-popup-trap');
      trap.type = 'text';
      trap.name = 'website';
      trap.tabIndex = -1;
      trap.setAttribute('autocomplete', 'off');
      trap.setAttribute('aria-hidden', 'true');
      form.appendChild(trap);

      var submit = el('button', 'gh-popup-submit', p.button_label || 'Subscribe');
      submit.type = 'submit';
      form.appendChild(submit);

      /* The site's consent note, placed with textContent like everything
       * else the server sends. */
      if (site.consent) form.appendChild(el('p', 'gh-popup-consent', site.consent));

      var error = el('p', 'gh-popup-error');
      error.hidden = true;
      error.setAttribute('role', 'alert');
      form.appendChild(error);

      var opened = Date.now();
      form.addEventListener('submit', function (e) {
        e.preventDefault();
        error.hidden = true;
        submit.disabled = true;
        var body =
          'email=' + encodeURIComponent(email.value) +
          '&name=' + encodeURIComponent(p.capture_name && nameInput ? nameInput.value : '') +
          '&website=' + encodeURIComponent(trap.value) +
          '&url=' + encodeURIComponent(location.pathname) +
          '&t=' + opened;
        fetch(base + '/popups/api/' + p.id + '/lead/', {
          method: 'POST',
          credentials: 'same-origin',
          headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
          body: body,
        })
          .then(function (r) { return r.json().catch(function () { return {}; }); })
          .then(function (data) {
            submit.disabled = false;
            if (data && data.ok) {
              form.hidden = true;
              done.textContent = data.message || 'Thanks!';
              done.hidden = false;
              setTimeout(dismiss, 2600);
            } else {
              error.textContent = (data && data.message) || 'That could not be sent.';
              error.hidden = false;
            }
          })
          .catch(function () {
            submit.disabled = false;
            error.textContent = 'That could not be sent.';
            error.hidden = false;
          });
      });
      box.appendChild(form);
    } else if (p.button_url && p.button_label) {
      var link = el('a', 'gh-popup-submit', p.button_label);
      /* The URL came from the admin, not from a reader, but a link that can
       * reach back into the opener is worth closing off anyway. */
      link.href = p.button_url;
      link.rel = 'noopener';
      box.appendChild(link);
    }

    root.appendChild(box);
    document.body.appendChild(root);
    // One frame later, so the transition has a state to come from.
    requestAnimationFrame(function () { root.classList.add('gh-popup-in'); });

    function dismiss() {
      root.classList.remove('gh-popup-in');
      setTimeout(function () { if (root.parentNode) root.parentNode.removeChild(root); }, 220);
      document.removeEventListener('keydown', onKey);
    }
    function onKey(e) { if (e.key === 'Escape') dismiss(); }
    close.addEventListener('click', dismiss);
    document.addEventListener('keydown', onKey);
    // A click on the backdrop — the element itself, not the card inside it.
    if (p.kind === 'modal') {
      root.addEventListener('click', function (e) { if (e.target === root) dismiss(); });
    }
    var first = box.querySelector('input, a, button:not(.gh-popup-close)');
    if (first && p.kind === 'modal') first.focus({ preventScroll: true });
  }

  /* Wait for whatever this popup is waiting for. */
  function arm(p) {
    if (!due(p)) return;
    var fired = false;
    function go() {
      if (fired) return;
      fired = true;
      render(p);
    }
    if (p.trigger === 'scroll') {
      var want = p.trigger_value > 0 ? p.trigger_value : 50;
      var onScroll = function () {
        var h = document.documentElement;
        var max = (h.scrollHeight - h.clientHeight) || 1;
        if ((h.scrollTop || document.body.scrollTop) / max * 100 >= want) {
          window.removeEventListener('scroll', onScroll);
          go();
        }
      };
      window.addEventListener('scroll', onScroll, { passive: true });
      onScroll();
      return;
    }
    if (p.trigger === 'exit') {
      /* There is no "pointer left the window" on a touch screen. Rather than
       * never showing on a phone, exit intent falls back to a generous
       * delay — the reader who is about to leave a page on a phone is the
       * one who has been there a while. */
      var touch = !window.matchMedia || window.matchMedia('(pointer: coarse)').matches;
      if (touch) {
        setTimeout(go, 45000);
        return;
      }
      var onOut = function (e) {
        if (e.clientY <= 0) {
          document.removeEventListener('mouseout', onOut);
          go();
        }
      };
      document.addEventListener('mouseout', onOut);
      return;
    }
    setTimeout(go, (p.trigger_value > 0 ? p.trigger_value : 5) * 1000);
  }

  function start() {
    /* The trailing slash is deliberate: the site redirects the slashless
     * form to this one with a year-long cache, so asking for it directly
     * saves a 301 on every page view that loads this script. */
    fetch(base + '/popups/api/active/?path=' + encodeURIComponent(location.pathname), {
      credentials: 'same-origin',
    })
      .then(function (r) { return r.ok ? r.json() : { popups: [] }; })
      .then(function (data) {
        if (data && data.site) site = data.site;
        /* "Hide on phones" is a screen-width question, so it is answered
         * here: the server cannot see the viewport. 640px is where a modal
         * stops leaving any of the page visible around it. */
        if (site.hide_on_mobile && window.matchMedia && window.matchMedia('(max-width: 640px)').matches) {
          return;
        }
        var list = (data && data.popups) || [];
        for (var i = 0; i < list.length; i++) arm(list[i]);
      })
      .catch(function () {});
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', start);
  } else {
    start();
  }
})();
