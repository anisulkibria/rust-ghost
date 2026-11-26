/* Podcast editor conveniences (episode and show editors, the shows list).
 *
 * Every form posts and saves without it; the server re-reads and re-checks
 * everything (vuuut_core::podcast). What this adds:
 *   - uploading the audio through the admin's media endpoint, then filling
 *     in the address, the length (read from the file) and the size;
 *   - reading the length of audio pasted as an address;
 *   - adding and removing chapter rows;
 *   - loading a .vtt/.srt/.txt transcript file into the textarea;
 *   - copying feed addresses.
 * Text is placed with textContent and .value only.
 */
(function () {
  'use strict';
  function ready(fn) { if (document.readyState === 'loading') { document.addEventListener('DOMContentLoaded', fn); } else { fn(); } }
  function $(id) { return document.getElementById(id); }
  function clock(s) {
    s = Math.max(0, Math.round(s || 0));
    var h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), x = s % 60;
    return (h ? h + ':' + (m < 10 ? '0' : '') : '') + m + ':' + (x < 10 ? '0' : '') + x;
  }
  function changed(el) { el.dispatchEvent(new Event('input', { bubbles: true })); }

  ready(function () {
    audio();
    chapters();
    transcript();
    copy();
  });

  function audio() {
    var box = $('pc-audio');
    if (!box) { return; }
    var file = $('pc-audio-file'), url = $('pc-audio-url'), dur = $('pc-duration'), bytes = $('pc-bytes'), status = $('pc-audio-status'), preview = $('pc-audio-preview');
    function measure(src) {
      if (!src) { return; }
      var probe = new Audio();
      probe.preload = 'metadata';
      probe.addEventListener('loadedmetadata', function () {
        if (isFinite(probe.duration) && probe.duration > 0 && (!dur.value || dur.dataset.auto === '1')) {
          dur.value = clock(probe.duration); dur.dataset.auto = '1'; changed(dur);
        }
      });
      probe.src = src;
    }
    function show(src) { if (preview) { preview.src = src; preview.hidden = !src; } }
    if (url) {
      url.addEventListener('change', function () { show(url.value.trim()); measure(url.value.trim()); });
    }
    if (dur) { dur.addEventListener('input', function (e) { if (e.isTrusted) { dur.dataset.auto = '0'; } }); }
    if (!file) { return; }
    file.addEventListener('change', function () {
      var f = file.files && file.files[0];
      if (!f) { return; }
      if (bytes) { bytes.value = f.size; changed(bytes); }
      var local = URL.createObjectURL(f);
      dur.dataset.auto = '1';
      measure(local);
      status.textContent = 'Uploading ' + f.name + '…';
      var data = new FormData();
      data.append('file', f, f.name);
      data.append('ref', f.name);
      var xhr = new XMLHttpRequest();
      xhr.open('POST', box.getAttribute('data-upload'));
      xhr.withCredentials = true;
      xhr.upload.onprogress = function (e) { if (e.lengthComputable) { status.textContent = 'Uploading ' + f.name + ' — ' + Math.round(e.loaded / e.total * 100) + '%'; } };
      xhr.onload = function () {
        var out = null;
        try { out = JSON.parse(xhr.responseText); } catch (e) { out = null; }
        var u = out && out.media && out.media[0] && out.media[0].url;
        if (xhr.status >= 200 && xhr.status < 300 && u) {
          // Store the site-relative address, so a changed domain does not break it.
          try { var p = new URL(u, location.href); if (p.origin === location.origin) { u = p.pathname; } } catch (e) {}
          url.value = u; changed(url);
          show(local);
          status.textContent = 'Uploaded ' + f.name;
        } else {
          var msg = out && out.errors && out.errors[0] && out.errors[0].message;
          status.textContent = 'Upload failed' + (msg ? ': ' + msg : '');
        }
      };
      xhr.onerror = function () { status.textContent = 'Upload failed — check your connection.'; };
      xhr.send(data);
    });
  }

  function chapters() {
    var list = $('pc-chapters'), add = $('pc-chapter-add'), tpl = $('pc-chapter-tpl');
    if (!list || !add || !tpl) { return; }
    add.addEventListener('click', function () {
      var row = tpl.content.firstElementChild.cloneNode(true);
      // Start the new chapter where the player preview is, if it is playing.
      var pv = $('pc-audio-preview');
      if (pv && pv.currentTime > 0) { row.querySelector('[name=ch_time]').value = clock(pv.currentTime); }
      list.appendChild(row);
      row.querySelector('[name=ch_title]').focus();
      changed(list);
    });
    list.addEventListener('click', function (e) {
      var b = e.target.closest('[data-pc-remove]');
      if (b) { b.closest('.pc-chapter').remove(); changed(list); }
    });
  }

  function transcript() {
    var file = $('pc-transcript-file'), ta = $('pc-transcript');
    if (!file || !ta) { return; }
    file.addEventListener('change', function () {
      var f = file.files && file.files[0];
      if (!f) { return; }
      if (f.size > 600000) { alert('That transcript is too long (600 KB at most).'); return; }
      var r = new FileReader();
      r.onload = function () { ta.value = String(r.result || ''); changed(ta); };
      r.readAsText(f);
    });
  }

  function copy() {
    document.addEventListener('click', function (e) {
      var b = e.target.closest && e.target.closest('[data-pc-copy]');
      if (!b) { return; }
      e.preventDefault();
      var text = b.getAttribute('data-pc-copy');
      var done = function () { var was = b.textContent; b.textContent = 'Copied'; setTimeout(function () { b.textContent = was; }, 1500); };
      if (navigator.clipboard) { navigator.clipboard.writeText(text).then(done, function () {}); }
    });
  }
})();
