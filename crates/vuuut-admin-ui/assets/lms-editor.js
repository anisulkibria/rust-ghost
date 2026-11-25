/* Tools → LMS & Online Courses: the course and lesson editors' own script.
 * The Koenig canvas, cover image, search preview and "Unsaved changes" come
 * from ecom-editor.js, which these pages load first. What is here:
 *  - the curriculum builder: sections and lessons reordered by dragging or
 *    with the arrow buttons, written into the hidden `curriculum` field as
 *    JSON when the course is saved;
 *  - the access cards follow the chosen kind of access;
 *  - the quiz question builder, written into the hidden `questions` field;
 *  - lesson downloads: add a link row, upload a file, remove a row.
 * Without it the forms still save; only reordering and questions need it.
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
  function changed(form) { if (form) { form.dispatchEvent(new Event('change', {bubbles: true})); } }
  function basePath() {
    try { return JSON.parse(($('koenig-mount') || {dataset: {}}).dataset.config || '{}').basePath || ''; } catch (e) { return ''; }
  }

  ready(function () {
    course();
    lesson();
  });

  /* ----------------------------------------------------------- course */
  function course() {
    var form = $('lm-course');
    if (!form) { return; }

    // Access kind → which pricing fields show.
    function access() {
      var c = form.querySelector('input[name=access]:checked');
      form.className = form.className.replace(/\blm-access-\w+/g, '').trim() + ' lm-access-' + (c ? c.value : 'free');
    }
    form.querySelectorAll('input[name=access]').forEach(function (r) { r.addEventListener('change', access); });
    access();

    var cur = $('lm-cur');
    if (!cur) { return; }
    var dragged = null;

    cur.addEventListener('click', function (e) {
      var b = e.target.closest('button[data-move]');
      if (!b) { return; }
      e.preventDefault();
      var up = b.dataset.move === 'up';
      var item = b.closest('.lm-les') || b.closest('.lm-sec');
      if (item.classList.contains('lm-les')) {
        var sib = up ? item.previousElementSibling : item.nextElementSibling;
        if (sib) {
          item.parentNode.insertBefore(item, up ? sib : sib.nextElementSibling);
        } else {
          // Off the end of a section: into the neighbouring one.
          var sec = item.closest('.lm-sec');
          var other = up ? sec.previousElementSibling : sec.nextElementSibling;
          if (other && other.classList.contains('lm-sec')) {
            var list = other.querySelector('.lm-lessons');
            if (up) { list.appendChild(item); } else { list.insertBefore(item, list.firstChild); }
          }
        }
      } else {
        var s2 = up ? item.previousElementSibling : item.nextElementSibling;
        if (s2 && s2.classList.contains('lm-sec') && !s2.classList.contains('lm-sec-loose')) {
          cur.insertBefore(item, up ? s2 : s2.nextElementSibling);
        }
      }
      item.classList.add('lm-moved');
      setTimeout(function () { item.classList.remove('lm-moved'); }, 500);
      counts();
      changed(form);
    });

    cur.addEventListener('dragstart', function (e) {
      dragged = e.target.closest('.lm-les') || (e.target.closest('.lm-sec-head') && e.target.closest('.lm-sec'));
      if (!dragged) { return; }
      dragged.classList.add('dragging');
      e.dataTransfer.effectAllowed = 'move';
      try { e.dataTransfer.setData('text/plain', dragged.dataset.id || ''); } catch (x) { /* old browsers */ }
    });
    cur.addEventListener('dragend', function () {
      if (dragged) { dragged.classList.remove('dragging'); }
      dragged = null;
      counts();
    });
    cur.addEventListener('dragover', function (e) {
      if (!dragged) { return; }
      e.preventDefault();
      if (dragged.classList.contains('lm-les')) {
        var over = e.target.closest('.lm-les');
        var list = e.target.closest('.lm-sec') && e.target.closest('.lm-sec').querySelector('.lm-lessons');
        if (over && over !== dragged) {
          var r = over.getBoundingClientRect();
          over.parentNode.insertBefore(dragged, (e.clientY - r.top) > r.height / 2 ? over.nextElementSibling : over);
          changed(form);
        } else if (list && !over && list !== dragged.parentNode) {
          list.appendChild(dragged);
          changed(form);
        }
      } else {
        var sec = e.target.closest('.lm-sec');
        if (sec && sec !== dragged && !sec.classList.contains('lm-sec-loose')) {
          var rr = sec.getBoundingClientRect();
          cur.insertBefore(dragged, (e.clientY - rr.top) > rr.height / 2 ? sec.nextElementSibling : sec);
          changed(form);
        }
      }
    });
    // Section headers are draggable through their grip only, so the title stays editable.
    cur.querySelectorAll('.lm-sec-head .lm-grip').forEach(function (g) {
      var sec = g.closest('.lm-sec');
      g.addEventListener('mousedown', function () { sec.draggable = true; });
      g.addEventListener('mouseup', function () { sec.draggable = false; });
      sec.addEventListener('dragend', function () { sec.draggable = false; });
    });

    function counts() {
      cur.querySelectorAll('.lm-sec').forEach(function (s) {
        var n = s.querySelectorAll('.lm-les').length;
        var c = s.querySelector('.lm-sec-count');
        if (c) { c.textContent = n === 1 ? '1 item' : n + ' items'; }
      });
    }

    form.addEventListener('submit', function () {
      var plan = {sections: [], loose: []};
      cur.querySelectorAll('.lm-sec').forEach(function (s) {
        var ids = Array.prototype.map.call(s.querySelectorAll('.lm-les'), function (l) { return l.dataset.id; });
        if (!s.dataset.id) { plan.loose = plan.loose.concat(ids); return; }
        var t = s.querySelector('.lm-sec-title');
        plan.sections.push({id: s.dataset.id, title: t ? t.value : '', lessons: ids});
      });
      $('lm-curriculum').value = JSON.stringify(plan);
    });

    // Adding to the curriculum leaves this page; say so if there are unsaved edits.
    document.querySelectorAll('button[form^="lm-add-"], button[form^="lm-delsec-"]').forEach(function (b) {
      b.addEventListener('click', function (e) {
        var state = $('ec-save-state');
        if (state && /Unsaved/.test(state.textContent) && !window.confirm('You have unsaved changes to this course. Continue without saving them?')) {
          e.preventDefault();
        }
      });
    });
  }

  /* ----------------------------------------------------------- lesson */
  function lesson() {
    var form = $('lm-lesson');
    if (!form) { return; }

    form.querySelectorAll('input[name=kind]').forEach(function (r) {
      r.addEventListener('change', function () {
        form.className = form.className.replace(/\blm-kind-\w+/g, '').trim() + ' lm-kind-' + r.value;
        var note = $('lm-kind-note');
        if (!note) {
          note = el('p', {id: 'lm-kind-note', class: 'muted ec-hint', text: 'Save to switch the editor to this kind.'});
          r.closest('.lm-kinds').after(note);
        }
      });
    });

    // Downloads.
    var atts = $('lm-atts'), tpl = $('lm-att-tpl');
    function addRow(label, url) {
      if (!atts || !tpl) { return null; }
      var row = tpl.content.firstElementChild.cloneNode(true);
      row.querySelector('[name=att_label]').value = label || '';
      row.querySelector('[name=att_url]').value = url || '';
      atts.appendChild(row);
      changed(form);
      return row;
    }
    if (atts) {
      atts.addEventListener('click', function (e) {
        var b = e.target.closest('.lm-att-del');
        if (b) { b.closest('.lm-att').remove(); changed(form); }
      });
    }
    var addBtn = $('lm-att-add');
    if (addBtn) { addBtn.addEventListener('click', function () { var r = addRow('', ''); if (r) { r.querySelector('input').focus(); } }); }
    var file = $('lm-att-file');
    if (file) {
      file.addEventListener('change', function () {
        var f = file.files[0];
        if (!f) { return; }
        var row = addRow(f.name, 'Uploading…');
        var data = new FormData();
        data.append('file', f, f.name);
        data.append('ref', f.name);
        fetch(basePath() + '/ghost/api/admin/files/upload/', {method: 'POST', body: data, credentials: 'same-origin'})
          .then(function (r) { return r.json().then(function (b) { return {ok: r.ok, body: b}; }); })
          .then(function (res) {
            var list = res.body && (res.body.files || res.body.media || res.body.images);
            var url = list && list[0] && list[0].url;
            if (!res.ok || !url) { throw new Error('Upload failed'); }
            row.querySelector('[name=att_url]').value = url;
          })
          .catch(function () { row.querySelector('[name=att_url]').value = ''; row.querySelector('[name=att_url]').placeholder = 'Upload failed — paste a link instead'; });
        file.value = '';
      });
    }

    quiz(form);
  }

  /* ------------------------------------------------------ quiz builder */
  function quiz(form) {
    var mount = $('lm-quiz'), out = $('lm-questions');
    if (!mount || !out) { return; }
    var kinds = [];
    var qs = [];
    try { qs = JSON.parse(mount.dataset.questions || '[]'); kinds = JSON.parse(mount.dataset.kinds || '[]'); } catch (e) { qs = []; }
    qs.forEach(function (q) { q.options = q.options || []; });

    function blank(kind) {
      var q = {id: '', kind: kind || 'single', prompt: '', options: [], answer: '', explanation: '', points: 1};
      if (q.kind === 'single' || q.kind === 'multiple') { q.options = [{t: '', c: true}, {t: '', c: false}, {t: '', c: false}]; }
      if (q.kind === 'truefalse') { q.options = [{t: 'True', c: true}, {t: 'False', c: false}]; }
      return q;
    }
    function sync() { out.value = JSON.stringify(qs); changed(form); }

    function render() {
      mount.innerHTML = '';
      if (!qs.length) {
        mount.appendChild(el('p', {class: 'muted lm-q-empty', text: 'No questions yet. Add the first one below.'}));
      }
      qs.forEach(function (q, i) { mount.appendChild(card(q, i)); });
      var bar = el('div', {class: 'lm-q-add'}, [el('span', {class: 'muted', text: 'Add a question:'})]);
      kinds.forEach(function (k) {
        bar.appendChild(el('button', {type: 'button', class: 'btn tonal small', text: '+ ' + k.label, onclick: function () { qs.push(blank(k.key)); render(); sync(); var last = mount.querySelectorAll('.lm-q'); if (last.length) { last[last.length - 1].querySelector('textarea').focus(); } }}));
      });
      mount.appendChild(bar);
    }

    function card(q, i) {
      var box = el('div', {class: 'lm-q'});
      var kindSel = el('select', {class: 'ec-in', 'aria-label': 'Question type'});
      kinds.forEach(function (k) { var o = el('option', {value: k.key, text: k.label}); if (k.key === q.kind) { o.selected = true; } kindSel.appendChild(o); });
      kindSel.addEventListener('change', function () {
        var fresh = blank(kindSel.value);
        q.kind = fresh.kind;
        if ((q.kind === 'single' || q.kind === 'multiple') && q.options.length >= 2 && q.options[0].t !== 'True') { /* keep typed answers */ } else { q.options = fresh.options; }
        if (q.kind === 'single') { var seen = false; q.options.forEach(function (o) { if (o.c && !seen) { seen = true; } else { o.c = false; } }); }
        render(); sync();
      });
      var pts = el('input', {class: 'ec-in lm-q-pts', type: 'number', min: '1', max: '100', value: String(q.points || 1), 'aria-label': 'Points', title: 'Points'});
      pts.addEventListener('input', function () { q.points = parseInt(pts.value, 10) || 1; sync(); });
      var tools = el('span', {class: 'lm-q-tools'}, [
        el('button', {type: 'button', class: 'icon-btn', title: 'Move up', text: '↑', onclick: function () { if (i > 0) { qs.splice(i - 1, 0, qs.splice(i, 1)[0]); render(); sync(); } }}),
        el('button', {type: 'button', class: 'icon-btn', title: 'Move down', text: '↓', onclick: function () { if (i < qs.length - 1) { qs.splice(i + 1, 0, qs.splice(i, 1)[0]); render(); sync(); } }}),
        el('button', {type: 'button', class: 'icon-btn', title: 'Duplicate', text: '⧉', onclick: function () { var c = JSON.parse(JSON.stringify(q)); c.id = ''; qs.splice(i + 1, 0, c); render(); sync(); }}),
        el('button', {type: 'button', class: 'icon-btn lm-danger', title: 'Delete question', text: '×', onclick: function () { qs.splice(i, 1); render(); sync(); }})
      ]);
      box.appendChild(el('div', {class: 'lm-q-head'}, [el('b', {class: 'lm-q-n', text: String(i + 1)}), kindSel, el('label', {class: 'lm-q-ptsl'}, [pts, ' pts']), tools]));
      var prompt = el('textarea', {class: 'ec-in lm-q-prompt', rows: '2', maxlength: '1000', placeholder: 'Question'});
      prompt.value = q.prompt || '';
      prompt.addEventListener('input', function () { q.prompt = prompt.value; sync(); });
      box.appendChild(prompt);

      if (q.kind === 'text') {
        var ans = el('input', {class: 'ec-in', maxlength: '300', placeholder: 'Accepted answers, separated by |  — e.g. Paris|Paree'});
        ans.value = q.answer || '';
        ans.addEventListener('input', function () { q.answer = ans.value; sync(); });
        box.appendChild(ans);
      } else {
        var list = el('div', {class: 'lm-q-opts'});
        var name = 'lmq' + i + Math.random().toString(36).slice(2, 6);
        q.options.forEach(function (o, j) {
          var mark = el('input', {type: q.kind === 'multiple' ? 'checkbox' : 'radio', name: name, title: 'Right answer'});
          mark.checked = !!o.c;
          mark.addEventListener('change', function () {
            if (q.kind !== 'multiple') { q.options.forEach(function (x, k) { x.c = k === j; }); } else { o.c = mark.checked; }
            sync();
            render();
          });
          var t = el('input', {class: 'ec-in', maxlength: '300', placeholder: 'Answer ' + (j + 1)});
          t.value = o.t || '';
          if (q.kind === 'truefalse') { t.readOnly = true; }
          t.addEventListener('input', function () { o.t = t.value; sync(); });
          var row = el('label', {class: 'lm-q-opt' + (o.c ? ' right' : '')}, [mark, t]);
          if (q.kind !== 'truefalse' && q.options.length > 2) {
            row.appendChild(el('button', {type: 'button', class: 'icon-btn', title: 'Remove answer', text: '×', onclick: function (e) { e.preventDefault(); q.options.splice(j, 1); render(); sync(); }}));
          }
          list.appendChild(row);
        });
        box.appendChild(list);
        if (q.kind !== 'truefalse' && q.options.length < 12) {
          box.appendChild(el('button', {type: 'button', class: 'btn text small', text: '+ Add an answer', onclick: function () { q.options.push({t: '', c: false}); render(); sync(); }}));
        }
      }
      var ex = el('input', {class: 'ec-in lm-q-ex', maxlength: '1000', placeholder: 'Explanation shown after answering (optional)'});
      ex.value = q.explanation || '';
      ex.addEventListener('input', function () { q.explanation = ex.value; sync(); });
      box.appendChild(ex);
      return box;
    }

    render();
    form.addEventListener('submit', function () { out.value = JSON.stringify(qs); });
  }
})();
