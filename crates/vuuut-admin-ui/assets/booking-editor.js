/* Tools → Booking: the service, staff and settings screens' own script.
 * The Koenig canvas, the picture manager and "Unsaved changes" come from
 * ecom-editor.js where a page loads it. What is here:
 *  - the intake-question builder, written into the hidden `questions` field;
 *  - weekly hours: times show only for open days, a second shift on
 *    request, and Monday can be copied to the other days.
 * Without it the forms still save; only the question builder needs it.
 */
(function () {
  function ready(fn) { if (document.readyState !== 'loading') { fn(); } else { document.addEventListener('DOMContentLoaded', fn); } }
  function $(id) { return document.getElementById(id); }
  function el(tag, attrs, kids) {
    var n = document.createElement(tag);
    Object.keys(attrs || {}).forEach(function (k) {
      if (k === 'text') { n.textContent = attrs[k]; } else if (k === 'class') { n.className = attrs[k]; } else if (k.slice(0, 2) === 'on') { n.addEventListener(k.slice(2), attrs[k]); } else { n.setAttribute(k, attrs[k]); }
    });
    (kids || []).forEach(function (c) { if (c) { n.appendChild(typeof c === 'string' ? document.createTextNode(c) : c); } });
    return n;
  }

  ready(function () {
    questions();
    hours();
  });

  function questions() {
    var mount = $('bk-questions'), out = $('bk-questions-input');
    if (!mount || !out) { return; }
    var form = out.form;
    var qs = [];
    try { qs = JSON.parse(mount.dataset.questions || '[]'); } catch (e) { qs = []; }
    function sync() { out.value = JSON.stringify(qs); if (form) { form.dispatchEvent(new Event('change', {bubbles: true})); } }
    function render() {
      mount.innerHTML = '';
      qs.forEach(function (q, i) {
        var label = el('input', {class: 'ec-in', maxlength: '160', placeholder: 'Question — e.g. Any allergies?'});
        label.value = q.label || '';
        label.addEventListener('input', function () { q.label = label.value; sync(); });
        var kind = el('select', {class: 'ec-in', 'aria-label': 'Answer type'});
        [['text', 'Short answer'], ['long', 'Paragraph'], ['yesno', 'Yes / no']].forEach(function (k) {
          var o = el('option', {value: k[0], text: k[1]}); if ((q.kind || 'text') === k[0]) { o.selected = true; } kind.appendChild(o);
        });
        kind.addEventListener('change', function () { q.kind = kind.value; sync(); });
        var req = el('input', {type: 'checkbox'});
        req.checked = !!q.required;
        req.addEventListener('change', function () { q.required = req.checked; sync(); });
        mount.appendChild(el('div', {class: 'bk-q'}, [
          el('span', {class: 'bk-q-n', text: String(i + 1)}), label, kind,
          el('label', {class: 'ec-check bk-q-req'}, [req, ' Required']),
          el('button', {type: 'button', class: 'icon-btn', title: 'Move up', text: '↑', onclick: function () { if (i > 0) { qs.splice(i - 1, 0, qs.splice(i, 1)[0]); render(); sync(); } }}),
          el('button', {type: 'button', class: 'icon-btn lm-danger', title: 'Remove', text: '×', onclick: function () { qs.splice(i, 1); render(); sync(); }})
        ]));
      });
      if (qs.length < 12) {
        mount.appendChild(el('button', {type: 'button', class: 'btn tonal small', text: '+ Add a question', onclick: function () { qs.push({label: '', kind: 'text', required: false}); render(); sync(); var ins = mount.querySelectorAll('.bk-q input.ec-in'); if (ins.length) { ins[ins.length - 1].focus(); } }}));
      }
    }
    render();
    if (form) { form.addEventListener('submit', function () { qs = qs.filter(function (q) { return (q.label || '').trim(); }); out.value = JSON.stringify(qs); }); }
  }

  function hours() {
    var days = Array.prototype.slice.call(document.querySelectorAll('.dr-day'));
    if (!days.length) { return; }
    function paint(row) {
      var sel = row.querySelector('select');
      row.setAttribute('data-state', sel.value);
      var t = row.querySelectorAll('input[type=time]');
      if (sel.value === 'open' && t[0] && !t[0].value) { t[0].value = '09:00'; t[1].value = '17:00'; }
    }
    days.forEach(function (row) {
      row.querySelector('select').addEventListener('change', function () { paint(row); });
      var add = row.querySelector('.dr-add-shift');
      add.addEventListener('click', function () { row.querySelector('.dr-shift2').hidden = false; add.hidden = true; });
    });
    function copy(targets) {
      var src = days[0];
      var vals = Array.prototype.map.call(src.querySelectorAll('select, input[type=time]'), function (x) { return x.value; });
      var two = !src.querySelector('.dr-shift2').hidden;
      targets.forEach(function (row) {
        Array.prototype.forEach.call(row.querySelectorAll('select, input[type=time]'), function (x, i) { x.value = vals[i]; });
        row.querySelector('.dr-shift2').hidden = !two;
        row.querySelector('.dr-add-shift').hidden = two;
        paint(row);
      });
      var f = src.closest('form'); if (f) { f.dispatchEvent(new Event('change', {bubbles: true})); }
    }
    var wk = $('dr-copy-weekdays'), all = $('dr-copy-all');
    if (wk) { wk.addEventListener('click', function () { copy(days.slice(1, 5)); }); }
    if (all) { all.addEventListener('click', function () { copy(days.slice(1)); }); }
  }
})();
