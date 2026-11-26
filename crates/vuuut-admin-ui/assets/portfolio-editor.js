/* Portfolio project editor conveniences.
 *
 * The form posts and saves without it; the server rebuilds everything
 * (vuuut_core::portfolio::ProjectInput). What this adds:
 *   - adding and removing result and credit rows ([data-pf-add] clones the
 *     <template> named by data-pf-tpl into the element named by data-pf-add);
 *   - the accent colour picker, which writes the hidden `accent` field, and
 *     its "no accent" box, which clears it.
 */
(function () {
  'use strict';
  function ready(fn) { if (document.readyState === 'loading') { document.addEventListener('DOMContentLoaded', fn); } else { fn(); } }
  function changed(el) { el.dispatchEvent(new Event('input', { bubbles: true })); }

  ready(function () {
    document.addEventListener('click', function (e) {
      var add = e.target.closest && e.target.closest('[data-pf-add]');
      if (add) {
        var list = document.getElementById(add.getAttribute('data-pf-add'));
        var tpl = document.getElementById(add.getAttribute('data-pf-tpl'));
        if (list && tpl) {
          var row = tpl.content.firstElementChild.cloneNode(true);
          list.appendChild(row);
          var first = row.querySelector('input');
          if (first) { first.focus(); }
          changed(list);
        }
        return;
      }
      var rm = e.target.closest && e.target.closest('[data-pf-remove]');
      if (rm) {
        var r = rm.parentElement;
        var parent = r.parentElement;
        r.remove();
        if (parent) { changed(parent); }
      }
    });

    var pick = document.querySelector('[data-pf-accent]');
    var hidden = document.getElementById('pf-accent');
    var none = document.getElementById('pf-accent-none');
    if (pick && hidden) {
      pick.addEventListener('input', function () { hidden.value = pick.value; if (none) { none.checked = false; } changed(hidden); });
      if (none) { none.addEventListener('change', function () { hidden.value = none.checked ? '' : pick.value; changed(hidden); }); }
    }
  });
})();
