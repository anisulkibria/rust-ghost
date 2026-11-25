/* The public booking pages: the picker without page reloads, the visitor's
 * time zone with their booking, and a confirm before cancelling. Every link
 * and form works without it. */
(function () {
  function ready(fn) { if (document.readyState !== 'loading') { fn(); } else { document.addEventListener('DOMContentLoaded', fn); } }
  ready(function () {
    var tz = '';
    try { tz = Intl.DateTimeFormat().resolvedOptions().timeZone || ''; } catch (e) { tz = ''; }
    document.querySelectorAll('[data-tz]').forEach(function (i) { i.value = tz; });

    document.addEventListener('click', function (e) {
      var a = e.target.closest('a[data-pick]');
      if (!a) { return; }
      var picker = a.closest('[data-picker]');
      if (!picker || !window.fetch) { return; }
      e.preventDefault();
      var src = picker.getAttribute('data-picker');
      var query = a.getAttribute('href').split('?')[1] || '';
      var url = src + (src.indexOf('?') >= 0 ? '&' : '?') + query;
      picker.classList.add('vk-loading');
      fetch(url, {credentials: 'same-origin'})
        .then(function (r) { if (!r.ok) { throw new Error(); } return r.text(); })
        .then(function (html) {
          var box = document.createElement('div');
          box.innerHTML = html;
          var fresh = box.firstElementChild;
          picker.replaceWith(fresh);
          if (history.replaceState) { history.replaceState(null, '', a.getAttribute('href')); }
          var times = fresh.querySelector('.vk-times');
          if (times && a.classList.contains('vk-day') && times.getBoundingClientRect().top > window.innerHeight) { times.scrollIntoView({behavior: 'smooth', block: 'center'}); }
        })
        .catch(function () { window.location.href = a.href; });
    });

    document.addEventListener('change', function (e) {
      if (e.target.name === 'at') {
        var f = e.target.form;
        var d = f && f.querySelector('.vk-details');
        if (d) { d.classList.add('vk-ready'); if (d.getBoundingClientRect().top > window.innerHeight - 120) { d.scrollIntoView({behavior: 'smooth', block: 'start'}); } }
      }
    });

    document.querySelectorAll('form[data-confirm-cancel]').forEach(function (f) {
      f.addEventListener('submit', function (e) { if (!window.confirm('Cancel this booking? This cannot be undone.')) { e.preventDefault(); } });
    });
    document.querySelectorAll('form[data-booking-form]').forEach(function (f) {
      f.addEventListener('submit', function (e) {
        if (!f.querySelector('input[name=at]:checked')) { e.preventDefault(); var t = f.querySelector('.vk-times, .vk-cal'); if (t) { t.scrollIntoView({behavior: 'smooth', block: 'center'}); t.classList.add('vk-shake'); setTimeout(function () { t.classList.remove('vk-shake'); }, 600); } return; }
        var b = f.querySelector('button[type=submit]');
        if (b) { b.disabled = true; b.textContent = 'One moment…'; }
      });
    });
  });
})();
