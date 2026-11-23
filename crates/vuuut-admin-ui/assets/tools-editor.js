// Row editing for the Redirects and Routes tools: add, remove and reorder
// rows (order is meaning in both — the first matching rule or collection
// wins), and keep each row's badge honest while it is being typed into.
// The forms post the rows in DOM order, so moving a row here is all a
// reorder needs.
(function () {
  function rows(list) {
    return Array.prototype.filter.call(list.children, function (el) { return el.hasAttribute('data-row'); });
  }
  function renumber(list) {
    rows(list).forEach(function (row, i) {
      var num = row.querySelector('[data-num]');
      if (num) num.textContent = i + 1;
    });
  }
  function sibling(row, dir) {
    var el = dir === 'up' ? row.previousElementSibling : row.nextElementSibling;
    while (el && !el.hasAttribute('data-row')) el = dir === 'up' ? el.previousElementSibling : el.nextElementSibling;
    return el;
  }

  document.addEventListener('click', function (e) {
    var add = e.target.closest('[data-add-row]');
    if (add) {
      var tpl = document.getElementById(add.dataset.addRow);
      var list = document.getElementById(add.dataset.target);
      if (!tpl || !list) return;
      var row = tpl.content.firstElementChild.cloneNode(true);
      // before the empty-state block, which always sits last
      var empty = list.querySelector(':scope > .rx-empty');
      list.insertBefore(row, empty);
      renumber(list);
      var first = row.querySelector('input:not([type=hidden]), select');
      if (first) first.focus();
      return;
    }
    var remove = e.target.closest('[data-remove-row]');
    if (remove) {
      var gone = remove.closest('[data-row]');
      var parent = gone.parentNode;
      var next = sibling(gone, 'down') || sibling(gone, 'up');
      gone.remove();
      renumber(parent);
      var focus = next && next.querySelector('[data-remove-row]');
      if (focus) focus.focus();
      return;
    }
    var move = e.target.closest('[data-move]');
    if (move) {
      var r = move.closest('[data-row]');
      var other = sibling(r, move.dataset.move);
      if (!other) return;
      if (move.dataset.move === 'up') r.parentNode.insertBefore(r, other);
      else r.parentNode.insertBefore(other, r);
      renumber(r.parentNode);
      move.focus();
      r.classList.remove('rx-moved'); void r.offsetWidth; r.classList.add('rx-moved');
    }
  });

  document.addEventListener('input', function (e) {
    var t = e.target;
    if (t.matches('[data-from]')) {
      var badge = t.closest('[data-row]').querySelector('[data-kind]');
      var pattern = /[\^$*+?()[\]{}|\\]/.test(t.value);
      badge.textContent = pattern ? 'Pattern' : 'Exact';
      badge.classList.toggle('pat', pattern);
    } else if (t.matches('[data-route-filter]')) {
      var kind = t.closest('[data-row]').querySelector('[data-route-kind]');
      var channel = t.value.trim() !== '';
      kind.textContent = channel ? 'Channel' : 'Template page';
      kind.classList.toggle('published', channel);
      kind.classList.toggle('tonal', !channel);
    }
  });

  document.querySelectorAll('.rx-list, .rt-list').forEach(renumber);
  var hit = document.querySelector('.rx-hit');
  if (hit) hit.scrollIntoView({ block: 'center', behavior: 'smooth' });
})();
