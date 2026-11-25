/* Tools → Directory: the listing editor's own behaviour. The Koenig
 * description, the photo manager, the search preview and "Unsaved changes"
 * come from ecom-editor.js, which this page loads first. What is here:
 *  - the classified price card follows the Kind choice;
 *  - each day of the opening hours shows its times only when it is open,
 *    a second shift on request, and Monday can be copied to other days;
 *  - the logo: upload, or type an address, with a preview;
 *  - "Find them on OpenStreetMap" searches for the address typed so far.
 * The form saves without any of it.
 */
(function () {
  function ready(fn) { if (document.readyState !== 'loading') { fn(); } else { document.addEventListener('DOMContentLoaded', fn); } }
  function $(id) { return document.getElementById(id); }

  ready(function () {
    var form = $('dr-listing');
    if (!form) { return; }
    function changed() { form.dispatchEvent(new Event('change', {bubbles: true})); }

    // Kind → classified price card.
    function kind() {
      var c = form.querySelector('input[name=kind][value=classified]');
      form.classList.toggle('is-classified', !!(c && c.checked));
    }
    form.querySelectorAll('input[name=kind]').forEach(function (r) { r.addEventListener('change', kind); });
    kind();

    // Opening hours.
    function paintDay(row) {
      var sel = row.querySelector('select');
      row.setAttribute('data-state', sel.value);
      var first = row.querySelector('input[type=time]');
      if (sel.value === 'open' && first && !first.value) {
        first.value = '09:00';
        row.querySelectorAll('input[type=time]')[1].value = '17:00';
      }
    }
    var days = Array.prototype.slice.call(form.querySelectorAll('.dr-day'));
    days.forEach(function (row) {
      row.querySelector('select').addEventListener('change', function () { paintDay(row); });
      var add = row.querySelector('.dr-add-shift');
      add.addEventListener('click', function () {
        row.querySelector('.dr-shift2').hidden = false;
        add.hidden = true;
      });
    });
    function copyFrom(targets) {
      var src = days[0];
      if (!src) { return; }
      var vals = Array.prototype.map.call(src.querySelectorAll('select, input[type=time]'), function (el) { return el.value; });
      var two = !src.querySelector('.dr-shift2').hidden;
      targets.forEach(function (row) {
        Array.prototype.forEach.call(row.querySelectorAll('select, input[type=time]'), function (el, i) { el.value = vals[i]; });
        row.querySelector('.dr-shift2').hidden = !two;
        row.querySelector('.dr-add-shift').hidden = two;
        paintDay(row);
      });
      changed();
    }
    var wk = $('dr-copy-weekdays'), all = $('dr-copy-all');
    if (wk) { wk.addEventListener('click', function () { copyFrom(days.slice(1, 5)); }); }
    if (all) { all.addEventListener('click', function () { copyFrom(days.slice(1)); }); }

    // Logo.
    var logoUrl = $('dr-logo-url'), logoFile = $('dr-logo-file'), preview = document.querySelector('.dr-logo-preview');
    function paintLogo() {
      if (!preview || !logoUrl) { return; }
      var v = logoUrl.value.trim();
      preview.innerHTML = '';
      if (/^(https?:\/\/|\/(?!\/))/.test(v)) {
        var img = document.createElement('img');
        img.src = v;
        img.alt = '';
        preview.appendChild(img);
      }
    }
    if (logoUrl) { logoUrl.addEventListener('input', paintLogo); }
    if (logoFile) {
      logoFile.addEventListener('change', function () {
        var file = logoFile.files[0];
        if (!file) { return; }
        var base = '';
        try { base = JSON.parse($('koenig-mount').dataset.config || '{}').basePath || ''; } catch (e) { base = ''; }
        var data = new FormData();
        data.append('file', file, file.name);
        data.append('purpose', 'image');
        if (preview) { preview.textContent = '…'; }
        fetch(base + '/ghost/api/admin/images/upload/', {method: 'POST', body: data, credentials: 'same-origin'})
          .then(function (r) { return r.json(); })
          .then(function (b) {
            var list = Array.isArray(b) ? b : (b && b.images);
            var url = list && list[0] && list[0].url;
            if (!url) { throw new Error('Upload failed'); }
            logoUrl.value = url;
            paintLogo();
            changed();
          })
          .catch(function () { if (preview) { preview.textContent = '!'; } });
        logoFile.value = '';
      });
    }

    // Coordinates lookup.
    var find = $('dr-find-coords');
    if (find) {
      find.addEventListener('click', function () {
        var parts = ['dr-address', 'dr-town', 'dr-country'].map(function (id) { var el = $(id); return el ? el.value.trim() : ''; }).filter(Boolean);
        find.href = 'https://www.openstreetmap.org/search?query=' + encodeURIComponent(parts.join(', '));
      });
    }
  });
})();
