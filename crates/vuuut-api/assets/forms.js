/* Form card behaviour: send the response without leaving the page, show the
 * server's verdict next to the field it is about, and quietly renew an
 * expired token.
 *
 * This is an enhancement, not the mechanism. Without it the form posts
 * normally and the server answers with a page; every check that matters is
 * made by the server either way. Text from the server is only ever written
 * with textContent.
 */
(function () {
  'use strict';

  function each(list, fn) { Array.prototype.forEach.call(list, fn); }

  function clearErrors(form) {
    each(form.querySelectorAll('.vf-field.is-invalid'), function (field) { field.classList.remove('is-invalid'); });
    each(form.querySelectorAll('.vf-error'), function (p) { p.hidden = true; p.textContent = ''; });
    each(form.querySelectorAll('[aria-invalid]'), function (el) { el.removeAttribute('aria-invalid'); });
  }

  function showErrors(form, errors) {
    var first = null;
    Object.keys(errors || {}).forEach(function (key) {
      each(form.querySelectorAll('[data-vf-field]'), function (field) {
        if (field.getAttribute('data-vf-field') !== key) { return; }
        field.classList.add('is-invalid');
        var p = field.querySelector('.vf-error');
        if (p) { p.textContent = String(errors[key]); p.hidden = false; }
        var control = field.querySelector('input, textarea, select');
        if (control) { control.setAttribute('aria-invalid', 'true'); first = first || control; }
      });
    });
    if (first && first.focus) { first.focus(); }
  }

  function status(form, message, kind) {
    var el = form.querySelector('.vf-status');
    if (!el) { return; }
    el.textContent = message || '';
    el.className = 'vf-status' + (kind ? ' is-' + kind : '');
    el.hidden = !message;
  }

  /* Where the server says to go, or link to, after a response. It is the
   * site's own setting, but it is still only followed if it is an http(s)
   * address — never javascript: or data:, which would run in this page. */
  function safeUrl(value) {
    if (typeof value !== 'string' || !value) { return null; }
    try {
      var url = new URL(value, window.location.href);
      return (url.protocol === 'http:' || url.protocol === 'https:') ? url.href : null;
    } catch (err) { return null; }
  }

  function showLink(form, link) {
    var el = form.querySelector('.vf-status');
    var href = link && safeUrl(link.url);
    if (!el || !href) { return; }
    var anchor = document.createElement('a');
    anchor.className = 'vf-link';
    anchor.href = href;
    anchor.rel = 'noopener';
    anchor.textContent = String(link.label || 'Open link');
    el.appendChild(document.createElement('br'));
    el.appendChild(anchor);
  }

  function wait(ms) { return new Promise(function (resolve) { setTimeout(resolve, ms); }); }

  function post(form) {
    var body = new URLSearchParams();
    new FormData(form).forEach(function (value, key) {
      if (typeof value === 'string') { body.append(key, value); }
    });
    return fetch(form.getAttribute('action'), {
      method: 'POST',
      body: body,
      headers: {Accept: 'application/json'},
      credentials: 'same-origin'
    }).then(function (response) {
      return response.json().catch(function () { return {}; }).then(function (json) {
        return {status: response.status, json: json || {}};
      });
    });
  }

  function renewToken(form) {
    return fetch(form.getAttribute('data-vf-token-url'), {headers: {Accept: 'application/json'}, credentials: 'same-origin'})
      .then(function (response) { return response.ok ? response.json() : {}; })
      .then(function (json) {
        var input = form.querySelector('input[name="_token"]');
        if (json && typeof json.token === 'string' && input) { input.value = json.token; return json; }
        return null;
      });
  }

  function send(form, retried) {
    return post(form).then(function (result) {
      var json = result.json;
      // A page that sat open (or cached) past its token's life: fetch a new
      // one and send again, once. The pause is the server's minimum age for
      // a token, which it reports alongside the token.
      if (json.code === 'token' && !retried) {
        return renewToken(form).then(function (renewed) {
          if (!renewed) { return result; }
          return wait(Math.max(0, Number(renewed.wait_ms) || 0) + 150).then(function () { return send(form, true); });
        });
      }
      return result;
    });
  }

  function wire(form) {
    if (form.dataset.vfWired) { return; }
    form.dataset.vfWired = '1';
    form.addEventListener('submit', function (event) {
      if (!window.fetch || !window.URLSearchParams || !window.FormData) { return; }
      event.preventDefault();
      var button = form.querySelector('.vf-submit');
      if (button && button.disabled) { return; }
      clearErrors(form);
      status(form, '', '');
      if (button) { button.disabled = true; }

      send(form, false).then(function (result) {
        var json = result.json;
        if (result.status >= 200 && result.status < 300 && json.ok) {
          form.reset();
          status(form, json.message || 'Thank you.', 'ok');
          var next = safeUrl(json.redirect);
          if (next) { window.location.assign(next); return; }
          showLink(form, json.link);
          // The token is spent; take a new one in case they write again.
          renewToken(form);
          return;
        }
        if (json.errors) { showErrors(form, json.errors); }
        status(form, json.message || 'Something went wrong. Please try again.', 'error');
      }).catch(function () {
        status(form, 'The form could not be sent. Check your connection and try again.', 'error');
      }).then(function () {
        if (button) { button.disabled = false; }
      });
    });
  }

  function start() { each(document.querySelectorAll('form[data-vf-form]'), wire); }
  if (document.readyState === 'loading') { document.addEventListener('DOMContentLoaded', start); } else { start(); }
})();
