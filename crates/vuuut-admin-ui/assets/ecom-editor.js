/* Tools → E-commerce & Online Stores: the admin's store script.
 *
 * Every form it touches posts and saves without it. What it adds:
 *  - the product description in Koenig (the post editor's canvas), written
 *    into a hidden input as Lexical on submit; the plain textarea is the
 *    fallback when the bundle cannot start;
 *  - the media manager: upload, add by URL, reorder, remove;
 *  - the variants table, redrawn live from the options, keeping what was
 *    typed into rows that still exist;
 *  - the search preview, the handle from the title, profit and margin;
 *  - collection condition rows, staff-order product rows, discount code
 *    generator, select-all and bulk delete confirmation;
 *  - "Unsaved changes" and a warning before leaving with them.
 */
(function () {
  function ready(fn) { if (document.readyState !== 'loading') { fn(); } else { document.addEventListener('DOMContentLoaded', fn); } }
  function $(id) { return document.getElementById(id); }
  function text(s) { var d = document.createElement('div'); d.textContent = s == null ? '' : String(s); return d.innerHTML; }

  ready(function () {
    var form = $('ec-product') || $('ec-collection') || $('ec-customer');
    var dirty = false;
    var state = $('ec-save-state');
    function markDirty() {
      if (dirty) { return; }
      dirty = true;
      if (state) { state.textContent = 'Unsaved changes'; }
    }
    if (form) {
      form.addEventListener('input', markDirty);
      form.addEventListener('change', markDirty);
      form.addEventListener('submit', function () { dirty = false; if (state) { state.textContent = 'Saving…'; } });
      window.addEventListener('beforeunload', function (e) { if (dirty) { e.preventDefault(); e.returnValue = ''; } });
    }

    mountKoenig(markDirty);
    media(markDirty);
    variants(markDirty);
    seo();
    margin();
    rules();
    orderLines();
    codes();
    bulk();
    digitalFiles();
    storeAddress();
    deliveryCountries();
  });

  /* ------------------------------------------------------------- Koenig */
  function mountKoenig(markDirty) {
    var mount = $('koenig-mount');
    var input = $('ec-lexical');
    var fallback = $('ec-desc-fallback');
    if (!mount || !input) { return; }
    var Koenig = window['@tryghost/koenig-lexical'];
    var React = window.React, ReactDOM = window.ReactDOM;
    if (!Koenig || !React || !ReactDOM || !window.GhostKoenig) { mount.hidden = true; return; }
    var config = {};
    try { config = JSON.parse(mount.dataset.config || '{}'); } catch (e) { config = {}; }
    var api = null;
    var first = null;
    var integrations = window.GhostKoenig.create(config, function (msg) { if (msg) { console.warn(msg); } });
    var e = React.createElement;
    function Editor() {
      return e('div', {className: 'koenig-react-editor koenig-lexical'},
        e(Koenig.KoenigComposer, {
          initialEditorState: config.lexical || undefined,
          fileUploader: integrations.fileUploader,
          cardConfig: integrations.useCardConfig(),
          onError: function (err) { console.error(err); },
          darkMode: document.documentElement.classList.contains('dark'),
          isTKEnabled: false
        },
        e(Koenig.KoenigEditor, {
          placeholderText: config.placeholder || 'Describe this product…',
          placeholderClassName: 'editor-body-placeholder',
          registerAPI: function (a) { api = a; },
          // Koenig fires onChange on load and on selection; only a real
          // change to the document marks the product as edited.
          onChange: function () {
            if (!api) { return; }
            var now = api.serialize();
            if (first === null) { first = now; return; }
            if (now !== first) { markDirty(); }
          }
        }))
      );
    }
    try {
      ReactDOM.createRoot(mount).render(e(Editor));
    } catch (err) {
      console.error(err);
      mount.hidden = true;
      return;
    }
    if (fallback) {
      fallback.hidden = true;
      var ta = fallback.querySelector('textarea');
      if (ta) { ta.disabled = true; }
    }
    var f = input.form;
    f.addEventListener('submit', function () {
      if (api) { input.value = api.serialize(); }
    });
  }

  /* -------------------------------------------------------------- media */
  function media(markDirty) {
    var box = $('ec-media');
    if (!box) { return; }
    var addTile = box.querySelector('.ec-media-add');
    var base = '';
    var mount = $('koenig-mount');
    try { base = JSON.parse(mount.dataset.config || '{}').basePath || ''; } catch (e) { base = ''; }

    function item(url) {
      var d = document.createElement('div');
      d.className = 'ec-media-item';
      d.draggable = true;
      d.innerHTML = '<img src="' + text(url) + '" alt=""><input type="hidden" name="image" value="' + text(url) + '">' +
        '<div class="ec-media-tools"><button type="button" data-move="-1" title="Move earlier">‹</button>' +
        '<button type="button" data-move="1" title="Move later">›</button><button type="button" data-remove title="Remove">×</button></div>';
      return d;
    }
    function changed() { markDirty(); refreshImageOptions(); }
    function add(url) { box.insertBefore(item(url), addTile); changed(); }

    box.addEventListener('click', function (e) {
      var b = e.target.closest('button');
      if (!b) { return; }
      var it = b.closest('.ec-media-item');
      if (b.hasAttribute('data-remove')) { it.remove(); changed(); return; }
      var dir = parseInt(b.dataset.move, 10);
      if (dir < 0 && it.previousElementSibling) { box.insertBefore(it, it.previousElementSibling); changed(); }
      if (dir > 0 && it.nextElementSibling && it.nextElementSibling !== addTile) { box.insertBefore(it.nextElementSibling, it); changed(); }
    });
    var dragged = null;
    box.addEventListener('dragstart', function (e) { dragged = e.target.closest('.ec-media-item'); if (dragged) { dragged.classList.add('dragging'); } });
    box.addEventListener('dragend', function () { if (dragged) { dragged.classList.remove('dragging'); } dragged = null; });
    box.addEventListener('dragover', function (e) {
      if (!dragged) { return; }
      e.preventDefault();
      var over = e.target.closest('.ec-media-item');
      if (over && over !== dragged) {
        var r = over.getBoundingClientRect();
        box.insertBefore(dragged, (e.clientX - r.left) > r.width / 2 ? over.nextElementSibling : over);
        changed();
      }
    });

    var up = $('ec-upload');
    if (up) {
      up.addEventListener('change', function () {
        Array.prototype.forEach.call(up.files, function (file) {
          var ph = document.createElement('div');
          ph.className = 'ec-media-item ec-uploading';
          ph.innerHTML = '<span>Uploading…</span>';
          box.insertBefore(ph, addTile);
          var data = new FormData();
          data.append('file', file, file.name);
          data.append('purpose', 'image');
          fetch(base + '/ghost/api/admin/images/upload/', {method: 'POST', body: data, credentials: 'same-origin'})
            .then(function (r) { return r.json().then(function (b) { return {ok: r.ok, body: b}; }); })
            .then(function (res) {
              var list = Array.isArray(res.body) ? res.body : (res.body && res.body.images);
              var url = list && list[0] && list[0].url;
              if (!res.ok || !url) { throw new Error((res.body && res.body.errors && res.body.errors[0] && res.body.errors[0].message) || 'Upload failed'); }
              ph.replaceWith(item(url));
              changed();
            })
            .catch(function (err) { ph.innerHTML = '<span>' + text(err.message) + '</span>'; ph.classList.add('ec-failed'); setTimeout(function () { ph.remove(); }, 4000); });
        });
        up.value = '';
      });
    }
    var urlIn = $('ec-media-url'), urlBtn = $('ec-media-url-add');
    if (urlIn && urlBtn) {
      var go = function () {
        var v = urlIn.value.trim();
        if (/^(https?:\/\/|\/(?!\/))/.test(v)) { add(v); urlIn.value = ''; }
      };
      urlBtn.addEventListener('click', go);
      urlIn.addEventListener('keydown', function (e) { if (e.key === 'Enter') { e.preventDefault(); go(); } });
    }
  }

  function imageUrls() {
    return Array.prototype.map.call(document.querySelectorAll('#ec-media input[name=image]'), function (i) { return i.value; });
  }
  function refreshImageOptions() {
    var urls = imageUrls();
    document.querySelectorAll('select[name=v_image]').forEach(function (sel) {
      var cur = sel.value;
      sel.innerHTML = '<option value="">—</option>' + urls.map(function (u, i) {
        return '<option value="' + text(u) + '"' + (u === cur ? ' selected' : '') + '>Image ' + (i + 1) + '</option>';
      }).join('');
    });
  }

  /* ----------------------------------------------------------- variants */
  function variants(markDirty) {
    var opts = $('ec-options');
    var body = $('ec-variants');
    var wrap = $('ec-variants-wrap');
    if (!opts || !body) { return; }
    var addBtn = $('ec-add-option');
    var card = $('ec-pricing');

    function options() {
      var out = [];
      opts.querySelectorAll('.ec-option').forEach(function (row) {
        if (row.hidden) { return; }
        var name = row.querySelector('[name^=opt_name_]').value.trim();
        var seen = {};
        var values = row.querySelector('[name^=opt_values_]').value.split(',').map(function (v) { return v.trim(); }).filter(function (v) {
          var k = v.toLowerCase();
          if (!v || seen[k]) { return false; }
          seen[k] = true;
          return true;
        });
        if (name && values.length) { out.push(values); }
      });
      return out;
    }
    function combos(list) {
      var out = [[]];
      list.forEach(function (values) {
        var next = [];
        out.forEach(function (p) { values.forEach(function (v) { if (next.length < 250) { next.push(p.concat([v])); } }); });
        out = next;
      });
      return out;
    }
    function snapshot() {
      var m = {};
      body.querySelectorAll('tr').forEach(function (tr) {
        var vals = {};
        tr.querySelectorAll('input,select').forEach(function (i) { vals[i.name] = i.value; });
        m[tr.dataset.key] = vals;
      });
      return m;
    }
    function redraw() {
      var list = options();
      var keep = snapshot();
      var price = (document.querySelector('[name=price]') || {}).value || '';
      var compare = (document.querySelector('[name=compare_at]') || {}).value || '';
      var qty = (document.querySelector('[name=qty]') || {}).value || '0';
      if (!list.length) {
        body.innerHTML = '';
        wrap.hidden = true;
        if (card) { card.classList.remove('has-variants'); }
        return;
      }
      wrap.hidden = false;
      if (card) { card.classList.add('has-variants'); }
      body.innerHTML = combos(list).map(function (c) {
        var key = c.join(' / ');
        var v = keep[key] || {v_id: '', v_price: price, v_compare: compare, v_sku: '', v_qty: qty, v_image: ''};
        return '<tr data-key="' + text(key) + '"><td><input type="hidden" name="v_key" value="' + text(key) + '"><input type="hidden" name="v_id" value="' + text(v.v_id) + '"><b>' + text(key) + '</b></td>' +
          '<td><input class="ec-in" name="v_price" value="' + text(v.v_price) + '" inputmode="decimal" aria-label="Price" placeholder="0.00"></td>' +
          '<td><input class="ec-in" name="v_compare" value="' + text(v.v_compare) + '" inputmode="decimal" aria-label="Compare-at price"></td>' +
          '<td><input class="ec-in" name="v_sku" value="' + text(v.v_sku) + '" aria-label="SKU" maxlength="80"></td>' +
          '<td><input class="ec-in ec-in-qty" name="v_qty" value="' + text(v.v_qty) + '" inputmode="numeric" aria-label="Quantity"></td>' +
          '<td><select class="ec-in" name="v_image" aria-label="Image" data-current="' + text(v.v_image) + '"></select></td></tr>';
      }).join('');
      refreshImageOptions();
      body.querySelectorAll('select[name=v_image]').forEach(function (s) { s.value = s.dataset.current || ''; });
      var hint = wrap.querySelector('.ec-hint');
      if (hint) { var n = body.children.length; hint.textContent = n + (n === 1 ? ' variant' : ' variants') + ' · Saving creates, keeps or removes variants to match the options.'; }
    }
    var t = null;
    opts.addEventListener('input', function () { clearTimeout(t); t = setTimeout(redraw, 250); });
    opts.addEventListener('click', function (e) {
      var b = e.target.closest('[data-remove-option]');
      if (!b) { return; }
      var row = b.closest('.ec-option');
      row.querySelectorAll('input').forEach(function (i) { i.value = ''; });
      if (opts.querySelectorAll('.ec-option:not([hidden])').length > 1) { row.hidden = true; }
      redraw();
      markDirty();
      if (addBtn) { addBtn.hidden = false; }
    });
    if (addBtn) {
      addBtn.addEventListener('click', function () {
        var hidden = opts.querySelector('.ec-option[hidden]');
        if (hidden) { hidden.hidden = false; hidden.querySelector('input').focus(); }
        var label = addBtn.querySelector('[data-label]');
        if (label) { label.textContent = 'Add another option'; }
        if (!opts.querySelector('.ec-option[hidden]')) { addBtn.hidden = true; }
      });
      if (!opts.querySelector('.ec-option[hidden]')) { addBtn.hidden = true; }
    }
    if (card && options().length) { card.classList.add('has-variants'); }
  }

  /* ---------------------------------------------------------------- SEO */
  function seo() {
    var title = $('ec-title'), handle = $('ec-handle');
    var st = $('ec-seo-title'), sd = $('ec-seo-desc');
    var pt = $('ec-serp-title'), pd = $('ec-serp-desc'), pu = $('ec-serp-url');
    if (!title || !handle) { return; }
    var auto = !handle.value;
    function slug(s) {
      return s.toLowerCase().normalize('NFKD').replace(/[̀-ͯ]/g, '').replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 80);
    }
    function paint() {
      if (auto) { handle.value = slug(title.value); }
      if (pt) { pt.textContent = (st && st.value) || title.value || 'Product title'; }
      if (pd) { pd.textContent = (sd && sd.value) || ''; }
      if (pu) { var b = pu.querySelector('b'); if (b) { b.textContent = handle.value || slug(title.value); } }
    }
    title.addEventListener('input', paint);
    handle.addEventListener('input', function () { auto = !handle.value; paint(); });
    if (st) { st.addEventListener('input', paint); }
    if (sd) { sd.addEventListener('input', paint); }
    paint();
  }

  /* ------------------------------------------------------------- margin */
  function margin() {
    var price = $('ec-price'), cost = $('ec-cost'), p = $('ec-profit'), m = $('ec-margin');
    if (!price || !cost || !p || !m) { return; }
    var cfg = {};
    try { cfg = JSON.parse($('koenig-mount').dataset.config || '{}'); } catch (e) { cfg = {}; }
    var sym = cfg.symbol || '';
    function num(v) { var n = parseFloat(String(v).replace(/[^0-9.]/g, '')); return isNaN(n) ? null : n; }
    function paint() {
      var a = num(price.value), c = num(cost.value);
      if (a === null || c === null || a <= 0) { p.textContent = '—'; m.textContent = '—'; return; }
      p.textContent = sym + (a - c).toFixed(2);
      m.textContent = Math.round(((a - c) / a) * 1000) / 10 + '%';
    }
    price.addEventListener('input', paint);
    cost.addEventListener('input', paint);
    paint();
  }

  /* --------------------------------------------- collection conditions */
  function rules() {
    var list = $('ec-rules'), tpl = $('ec-rule-template'), add = $('ec-add-rule');
    var form = $('ec-collection');
    if (form) {
      form.addEventListener('change', function (e) {
        if (e.target.name === 'kind') { form.classList.toggle('is-smart', e.target.value === 'smart'); }
      });
    }
    if (!list || !tpl || !add) { return; }
    add.addEventListener('click', function () { list.insertAdjacentHTML('beforeend', tpl.innerHTML); });
    list.addEventListener('click', function (e) {
      var b = e.target.closest('[data-remove-rule]');
      if (!b) { return; }
      if (list.children.length > 1) { b.closest('.ec-rule').remove(); } else { b.closest('.ec-rule').querySelector('input').value = ''; }
    });
  }

  /* --------------------------------------------------- staff order rows */
  function orderLines() {
    var add = $('ec-add-line'), list = $('ec-newlines'), tpl = $('ec-line-template');
    if (!add || !list || !tpl) { return; }
    add.addEventListener('click', function () {
      list.insertAdjacentHTML('beforeend', tpl.innerHTML);
      var spans = list.querySelectorAll('.ec-newline .field span:not(.car)');
      var n = 0;
      list.querySelectorAll('.ec-newline').forEach(function (row) { n += 1; var s = row.querySelector('.field span'); if (s) { s.textContent = 'Product ' + n; } });
      void spans;
    });
  }

  /* -------------------------------------------------------------- codes */
  function codes() {
    var b = $('ec-gen-code'), input = $('ec-code');
    if (!b || !input) { return; }
    b.addEventListener('click', function () {
      var a = 'ABCDEFGHJKLMNPQRSTUVWXYZ23456789', out = '';
      var bytes = new Uint8Array(10);
      (window.crypto || window.msCrypto).getRandomValues(bytes);
      bytes.forEach(function (x) { out += a[x % a.length]; });
      input.value = out;
    });
  }

  /* ------------------------------------------------- digital file picker */
  function digitalFiles() {
    var input = $('ec-digital-input') || $('ec-csv-input'), list = $('ec-digital-picked') || $('ec-csv-picked');
    // Remove buttons submit the product form to their own address; ask first.
    document.querySelectorAll('[data-confirm-click]').forEach(function (b) {
      b.addEventListener('click', function (e) { if (!window.confirm(b.dataset.confirmClick)) { e.preventDefault(); } });
    });
    if (!input || !list) { return; }
    var drop = input.closest('.ec-drop');
    function size(n) {
      var u = ['B', 'KB', 'MB', 'GB'], i = 0;
      while (n >= 1024 && i < u.length - 1) { n /= 1024; i++; }
      return (i === 0 ? n : n.toFixed(n >= 10 ? 0 : 1)) + ' ' + u[i];
    }
    function paint() {
      var files = Array.prototype.slice.call(input.files || []);
      list.innerHTML = files.map(function (f) {
        var big = f.size > 2 * 1024 * 1024 * 1024;
        return '<li class="' + (big ? 'too-big' : '') + '"><b>' + text(f.name) + '</b><span>' + size(f.size) + (big ? ' · over 2 GB, will be refused' : (input.id === 'ec-csv-input' ? ' · ready to import' : ' · uploads when you save')) + '</span></li>';
      }).join('');
      if (drop) { drop.classList.toggle('has-files', files.length > 0); }
    }
    input.addEventListener('change', paint);
    if (drop) {
      ['dragenter', 'dragover'].forEach(function (ev) { drop.addEventListener(ev, function (e) { e.preventDefault(); drop.classList.add('over'); }); });
      ['dragleave', 'drop'].forEach(function (ev) { drop.addEventListener(ev, function () { drop.classList.remove('over'); }); });
      drop.addEventListener('drop', function (e) {
        e.preventDefault();
        if (e.dataTransfer && e.dataTransfer.files.length) { input.files = e.dataTransfer.files; paint(); input.dispatchEvent(new Event('change', {bubbles: true})); }
      });
    }
    // A big upload takes a while: say so on the Save button.
    var form = input.form;
    if (form) {
      form.addEventListener('submit', function () {
        if (input.files && input.files.length) {
          var st = $('ec-save-state');
          if (st) { st.textContent = 'Uploading ' + input.files.length + (input.files.length === 1 ? ' file…' : ' files…'); }
        }
      });
    }
  }

  /* ------------------------------------------------------ store address */
  function storeAddress() {
    var box = document.querySelector('.ec-address');
    if (!box) { return; }
    var site = box.dataset.site || '';
    var base = $('ec-base-path'), product = $('ec-product-path');
    function clean(v, fallback) {
      var parts = String(v || '').toLowerCase().normalize('NFKD').replace(/[̀-ͯ]/g, '').split('/')
        .map(function (p) { return p.replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, ''); })
        .filter(function (p) { return p; }).slice(0, 3);
      return parts.length ? parts.join('/') : fallback;
    }
    function paint() {
      var b = clean(base.value, 'shop');
      var p = clean(product.value, b + '/products');
      var set = function (key, path) { var el = box.querySelector('[data-example="' + key + '"]'); if (el) { el.textContent = site + '/' + path + '/product-name/'; } };
      set('under', b + '/products');
      set('direct', b);
      set('custom', p);
      var ex = $('ec-base-example'); if (ex) { ex.textContent = site + '/' + b + '/cart/'; }
    }
    base.addEventListener('input', paint);
    product.addEventListener('input', paint);
    paint();
  }

  function deliveryCountries() {
    var filter = $('ec-country-filter');
    if (!filter) { return; }
    var options = $('ec-country-options');
    var summary = $('ec-country-summary');
    var labels = Array.from(options.querySelectorAll('label'));
    $('ec-country-filter-field').hidden = false;
    filter.addEventListener('input', function () {
      var query = filter.value.trim().toLowerCase();
      var found = false;
      labels.forEach(function (label) {
        label.hidden = !label.textContent.toLowerCase().includes(query);
        if (!label.hidden) { found = true; }
      });
      $('ec-country-no-results').hidden = found;
    });
    filter.addEventListener('keydown', function (event) {
      if (event.key === 'Enter') { event.preventDefault(); }
    });
    options.addEventListener('change', function () {
      var count = options.querySelectorAll('input:checked').length;
      summary.textContent = count === 0 ? 'All countries' : count + (count === 1 ? ' country selected' : ' countries selected');
    });
  }

  /* --------------------------------------------------------------- bulk */
  function bulk() {
    var all = document.querySelector('[data-check-all]');
    if (all) {
      all.addEventListener('change', function () {
        document.querySelectorAll('input[name=id][form=ec-bulk]').forEach(function (c) { c.checked = all.checked; });
      });
    }
    var f = $('ec-bulk');
    if (f) {
      f.addEventListener('submit', function (e) {
        var action = document.querySelector('select[name=action][form=ec-bulk]');
        if (action && action.value === 'delete') {
          var parts = (f.dataset.confirmDelete || '').split('|');
          if (!window.confirm(parts[1] || 'Delete the selected products?')) { e.preventDefault(); }
        }
      });
    }
  }
})();
