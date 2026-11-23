/* Find an entry to link as a translation.
 *
 * Drives the "Or link one that exists" box in the editor's Language panel:
 * type, see matching entries, click one to pick it.
 *
 * This is a convenience, not the authority. The server re-resolves whatever
 * the box posts and re-checks every rule — the entry exists, it is the same
 * kind, it is in a different language, that language is not already taken,
 * and the person may edit both sides. So the worst a tampered-with box can
 * do is get a refusal.
 *
 * It is also optional. With JavaScript off the box is still a working input:
 * type an entry's exact slug and the server resolves it. That is why the
 * input is a plain `name="target"` rather than a hidden field driven from
 * here — a dead input would be worse than no input.
 */
(function () {
  'use strict';

  var DEBOUNCE_MS = 200;

  function init(box) {
    var endpoint = box.getAttribute('data-xlate-search');
    var input = box.querySelector('[data-xlate-input]');
    var results = box.querySelector('[data-xlate-results]');
    if (!endpoint || !input || !results) {
      return;
    }

    var timer = null;
    var controller = null;
    // What the server told us about the entry currently chosen, so the box
    // can show a title while posting an id.
    var chosen = null;

    function close() {
      results.hidden = true;
      results.textContent = '';
    }

    function choose(entry) {
      chosen = entry;
      // The input posts the id; the person sees the title. Set with `value`,
      // never by writing markup.
      input.value = entry.id;
      input.setAttribute('data-xlate-label', entry.title);
      input.type = 'hidden';
      close();
      render_chosen();
    }

    function clear_chosen() {
      chosen = null;
      input.type = 'text';
      input.value = '';
      input.removeAttribute('data-xlate-label');
      var pill = box.querySelector('[data-xlate-chosen]');
      if (pill) {
        pill.remove();
      }
      input.focus();
    }

    function render_chosen() {
      var existing = box.querySelector('[data-xlate-chosen]');
      if (existing) {
        existing.remove();
      }
      if (!chosen) {
        return;
      }
      var pill = document.createElement('div');
      pill.setAttribute('data-xlate-chosen', '');
      pill.className = 'lg-chosen';
      var code = document.createElement('span');
      code.className = 'chip tonal';
      code.textContent = chosen.locale;
      var name = document.createElement('span');
      name.className = 'lg-chosen-title';
      name.textContent = chosen.title;
      var drop = document.createElement('button');
      drop.type = 'button';
      drop.className = 'btn text small';
      drop.textContent = 'Change';
      drop.addEventListener('click', clear_chosen);
      pill.appendChild(code);
      pill.appendChild(name);
      pill.appendChild(drop);
      input.parentNode.insertBefore(pill, input.parentNode.firstChild);
    }

    function render(entries, more) {
      results.textContent = '';
      if (!entries.length) {
        var none = document.createElement('p');
        none.className = 'lg-result-none';
        none.textContent = 'Nothing matches that in another language.';
        results.appendChild(none);
        show();
        return;
      }
      entries.forEach(function (entry) {
        var row = document.createElement('button');
        row.type = 'button';
        row.className = 'lg-result';
        var code = document.createElement('span');
        code.className = 'chip tonal';
        code.textContent = entry.locale;
        var title = document.createElement('span');
        title.className = 'lg-result-title';
        // Text from the database, placed as text. Never innerHTML.
        title.textContent = entry.title;
        var status = document.createElement('span');
        status.className = 'lg-result-status';
        status.textContent = entry.status;
        row.appendChild(code);
        row.appendChild(title);
        row.appendChild(status);
        row.addEventListener('click', function () {
          choose(entry);
        });
        results.appendChild(row);
      });
      // The server returns a capped list; say so rather than letting the
      // last row look like the last match.
      if (more) {
        var note = document.createElement('p');
        note.className = 'lg-result-more';
        note.textContent = 'More entries match. Type more of the title to narrow it down.';
        results.appendChild(note);
      }
      show();
    }

    // Open the list and bring it into view: the sidebar scrolls itself, and
    // the box is near the bottom of a panel near the bottom of it.
    function show() {
      var wasHidden = results.hidden;
      results.hidden = false;
      if (wasHidden && results.scrollIntoView) {
        results.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
      }
    }

    function rows() {
      return Array.prototype.slice.call(results.querySelectorAll('.lg-result'));
    }

    // Arrow keys move between the input and the results.
    results.addEventListener('keydown', function (event) {
      var list = rows();
      var at = list.indexOf(document.activeElement);
      if (event.key === 'ArrowDown' && at < list.length - 1) {
        event.preventDefault();
        list[at + 1].focus();
      } else if (event.key === 'ArrowUp') {
        event.preventDefault();
        if (at > 0) { list[at - 1].focus(); } else { input.focus(); }
      } else if (event.key === 'Escape') {
        close();
        input.focus();
      }
    });

    function search() {
      var term = input.value.trim();
      if (controller) {
        controller.abort();
      }
      controller = typeof AbortController === 'function' ? new AbortController() : null;
      fetch(endpoint + '?q=' + encodeURIComponent(term), {
        credentials: 'same-origin',
        headers: { Accept: 'application/json' },
        signal: controller ? controller.signal : undefined
      })
        .then(function (response) {
          return response.ok ? response.json() : { entries: [] };
        })
        .then(function (payload) {
          render((payload && payload.entries) || [], !!(payload && payload.more));
        })
        .catch(function () {
          /* An aborted or failed search leaves the box usable: a slug typed
             by hand still posts and still resolves on the server. */
          close();
        });
    }

    input.addEventListener('input', function () {
      if (timer) {
        window.clearTimeout(timer);
      }
      timer = window.setTimeout(search, DEBOUNCE_MS);
    });
    input.addEventListener('focus', function () {
      if (!chosen) {
        search();
      }
    });
    // Enter picks the first result rather than submitting a half-typed title
    // as if it were a slug.
    input.addEventListener('keydown', function (event) {
      if (event.key === 'Escape') {
        close();
        return;
      }
      if (event.key === 'ArrowDown' && !results.hidden) {
        var list = rows();
        if (list.length) {
          event.preventDefault();
          list[0].focus();
        }
        return;
      }
      if (event.key !== 'Enter') {
        return;
      }
      var first = results.querySelector('.lg-result');
      if (first && !results.hidden) {
        event.preventDefault();
        first.click();
      }
    });
    document.addEventListener('click', function (event) {
      if (!box.contains(event.target)) {
        close();
      }
    });
  }

  function start() {
    var boxes = document.querySelectorAll('[data-xlate-search]');
    for (var i = 0; i < boxes.length; i += 1) {
      init(boxes[i]);
    }
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', start);
  } else {
    start();
  }
})();
