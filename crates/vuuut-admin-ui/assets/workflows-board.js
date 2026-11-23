/*
 * The editorial board's drag and drop.
 *
 * Every card already has ← → buttons that are plain forms, so the board
 * works without this script. This adds dragging a card between columns: on
 * drop it posts the same form the arrows post, asking for JSON, and the
 * server — which decides who may move what where — answers. A refusal puts
 * the card back and says why; nothing is trusted to the browser.
 */
(function () {
  'use strict';
  var board = document.getElementById('wf-board');
  if (!board) return;
  var dragging = null, origin = null, next = null;

  function toast(text) {
    var n = document.createElement('div');
    n.className = 'wf-toast';
    n.setAttribute('role', 'status');
    n.textContent = text;
    document.body.appendChild(n);
    setTimeout(function () { n.remove(); }, 4000);
  }

  function count(col) {
    var c = col.querySelector('.wf-count');
    if (!c) return;
    var n = col.querySelectorAll('.wf-card').length;
    c.textContent = c.textContent.replace(/^\d+/, String(n));
    var empty = col.querySelector('.wf-empty');
    if (empty) empty.hidden = n > 0;
  }

  board.addEventListener('dragstart', function (e) {
    var card = e.target.closest && e.target.closest('.wf-card[draggable="true"]');
    if (!card) return;
    dragging = card; origin = card.parentNode; next = card.nextSibling;
    card.classList.add('wf-dragging');
    e.dataTransfer.effectAllowed = 'move';
    e.dataTransfer.setData('text/plain', card.getAttribute('data-id'));
  });
  board.addEventListener('dragend', function () {
    if (dragging) dragging.classList.remove('wf-dragging');
    Array.prototype.forEach.call(board.querySelectorAll('.wf-drop-on'), function (c) { c.classList.remove('wf-drop-on'); });
    dragging = null;
  });
  board.addEventListener('dragover', function (e) {
    var col = e.target.closest && e.target.closest('.wf-col[data-movable="true"]');
    if (!col || !dragging) return;
    e.preventDefault();
    Array.prototype.forEach.call(board.querySelectorAll('.wf-drop-on'), function (c) { if (c !== col) c.classList.remove('wf-drop-on'); });
    col.classList.add('wf-drop-on');
  });
  board.addEventListener('drop', function (e) {
    var col = e.target.closest && e.target.closest('.wf-col[data-movable="true"]');
    if (!col || !dragging) return;
    e.preventDefault();
    var card = dragging, from = origin, before = next;
    var id = card.getAttribute('data-id');
    var to = col.getAttribute('data-stage');
    if (col.contains(card)) return;
    var list = col.querySelector('[data-drop]');
    list.insertBefore(card, list.firstChild);
    count(col); count(from.closest('.wf-col'));
    var body = 'to=' + encodeURIComponent(to);
    fetch(board.baseURI.split('/ghost/')[0] + '/ghost/workflows/items/' + encodeURIComponent(id) + '/move/', {
      method: 'POST', credentials: 'same-origin',
      headers: { 'Content-Type': 'application/x-www-form-urlencoded', 'Accept': 'application/json' },
      body: body
    }).then(function (r) { return r.json().catch(function () { return { ok: false, message: 'That could not be saved.' }; }); })
      .then(function (data) {
        if (!data.ok) {
          from.insertBefore(card, before);
          count(col); count(from.closest('.wf-col'));
        }
        toast(data.message || (data.ok ? 'Moved.' : 'That could not be moved.'));
      })
      .catch(function () {
        from.insertBefore(card, before);
        count(col); count(from.closest('.wf-col'));
        toast('The board could not be reached.');
      });
  });
})();
