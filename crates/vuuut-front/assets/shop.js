/* The online store's small script. Everything works without it — the
 * variant <select>, the Update buttons — this only makes it quicker:
 *  - option pills choose the variant and update price, stock and image;
 *  - gallery thumbnails swap the main image;
 *  - changing a cart quantity submits the cart;
 *  - changing the delivery country re-prices shipping at checkout. */
(function () {
  function ready(fn) { if (document.readyState !== 'loading') { fn(); } else { document.addEventListener('DOMContentLoaded', fn); } }
  ready(function () {
    var main = document.getElementById('vs-main-img');
    document.querySelectorAll('.vs-thumb').forEach(function (b) {
      b.addEventListener('click', function () {
        if (!main) { return; }
        main.src = b.dataset.src;
        document.querySelectorAll('.vs-thumb').forEach(function (x) { x.classList.toggle('on', x === b); });
      });
    });

    var buy = document.querySelector('.vs-buy[data-variants]');
    if (buy) {
      var variants = [];
      try { variants = JSON.parse(buy.dataset.variants); } catch (e) { variants = []; }
      var select = document.getElementById('vs-variant');
      var sets = buy.querySelectorAll('.vs-opt');
      var add = document.getElementById('vs-add');
      var stock = document.getElementById('vs-stock');
      var priceRow = document.getElementById('vs-price-row');
      // Stay on the product after adding, unless the form already says
      // where to go (a free download goes straight to checkout).
      if (!buy.querySelector('input[name=back]')) {
        var back = document.createElement('input');
        back.type = 'hidden'; back.name = 'back'; back.value = 'product';
        buy.appendChild(back);
      }
      if (sets.length && select) { select.closest('label').hidden = true; }
      function chosen() {
        return Array.prototype.map.call(sets, function (fs) {
          var c = fs.querySelector('input:checked');
          return c ? c.value : '';
        });
      }
      function text(s) { var d = document.createElement('div'); d.textContent = s; return d.innerHTML; }
      function apply() {
        var values = chosen();
        var v = variants.find(function (x) { return x.options.every(function (o, i) { return o === values[i]; }); });
        sets.forEach(function (fs, i) {
          var label = fs.querySelector('[data-chosen]');
          if (label) { label.textContent = values[i]; }
          // Grey out values that, with the other choices kept, cannot be bought.
          fs.querySelectorAll('.vs-pill').forEach(function (pill) {
            var val = pill.querySelector('input').value;
            var probe = values.slice(); probe[i] = val;
            var ok = variants.some(function (x) { return x.available && x.options.every(function (o, j) { return o === probe[j]; }); });
            pill.classList.toggle('na', !ok);
          });
        });
        if (!v) {
          if (add) { add.disabled = true; add.textContent = 'Unavailable'; }
          if (stock) { stock.textContent = 'This combination is not available.'; }
          return;
        }
        if (select) { select.value = v.id; }
        priceRow.innerHTML = '<span class="vs-price vs-price-lg">' + text(v.price) + '</span>' +
          (v.compare ? ' <s class="vs-was">' + text(v.compare) + '</s> <span class="vs-tag">Sale</span>' : '');
        // no cart button when the product sells through a direct link
        if (add) {
          add.disabled = !v.available;
          add.textContent = v.available ? 'Add to cart' : 'Sold out';
        }
        if (stock) {
          stock.textContent = !v.available ? 'Sold out' : (v.low ? 'Only ' + v.stock + ' left' : (v.stock === 0 ? 'Available to pre-order' : 'In stock'));
        }
        if (v.image && main) { main.src = v.image; }
        try { history.replaceState(null, '', '?variant=' + encodeURIComponent(v.id)); } catch (e) { /* ignore */ }
      }
      sets.forEach(function (fs) { fs.addEventListener('change', apply); });
      if (sets.length) { apply(); }
    }

    document.querySelectorAll('input[data-autosubmit]').forEach(function (i) {
      var t = null;
      i.addEventListener('change', function () {
        clearTimeout(t);
        t = setTimeout(function () { i.form.requestSubmit ? i.form.requestSubmit() : i.form.submit(); }, 350);
      });
    });

    document.querySelectorAll('[data-reprice]').forEach(function (el) {
      el.addEventListener('change', function () {
        var f = el.form;
        var r = document.createElement('input');
        r.type = 'hidden'; r.name = 'reprice'; r.value = '1';
        f.appendChild(r);
        f.noValidate = true;
        f.submit();
      });
    });
  });
})();
