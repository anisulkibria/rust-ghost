/* The media library's progressive enhancement.
 *
 * Everything this file does, the page already does without it: the Upload
 * control is a real multipart form that posts to the same URL, selection is
 * CSS over real checkboxes, and the filters are a GET form. What it adds is
 * drag-and-drop, a progress line, and not having to press a second button
 * after choosing files.
 *
 * It is never the authority. The server re-derives the family each file
 * belongs to, re-checks the type against that family's allowlist, re-checks
 * the size against the family's ceiling and the host's limit, and composes the
 * stored name itself — see ghost_core::media and ghost_api::images::store.
 * This script's size check only exists to fail fast on a file that could never
 * be accepted, so a 2 GB video is refused before it is uploaded rather than
 * after.
 *
 * Text from the server or from a file name is placed with textContent only.
 */
(function () {
  'use strict';

  var form = document.querySelector('[data-ml-upload]');
  if (!form) return;
  var input = form.querySelector('input[type="file"]');
  var submit = form.querySelector('[data-ml-submit]');
  var state = form.querySelector('[data-ml-state]');
  var drop = document.querySelector('[data-ml-drop]');
  var config = {};
  try {
    config = JSON.parse((drop && drop.dataset.ml) || '{}');
  } catch (e) {
    config = {};
  }
  var uploadUrl = config.uploadUrl || form.getAttribute('action');
  var maxBytes = Number(config.maxBytes) || 0;
  var busy = false;

  // With the script running, choosing files is the whole gesture — the submit
  // button is what makes the no-JavaScript path work and has nothing to do
  // here. Removed rather than hidden so it cannot be reached by keyboard.
  if (submit) submit.remove();

  function say(message) {
    if (state) state.textContent = message;
  }

  function plural(n, word) {
    return n + ' ' + (n === 1 ? word : word + 's');
  }

  function send(files) {
    if (busy || !files || !files.length) return;
    var queue = [];
    var tooBig = [];
    for (var i = 0; i < files.length; i++) {
      if (maxBytes && files[i].size > maxBytes) tooBig.push(files[i].name);
      else queue.push(files[i]);
    }
    if (!queue.length) {
      say('Too large to upload: ' + tooBig.join(', '));
      return;
    }
    busy = true;
    if (drop) drop.classList.add('ml-busy');

    var data = new FormData();
    queue.forEach(function (file) { data.append('file', file); });

    var request = new XMLHttpRequest();
    request.open('POST', uploadUrl);
    // Same-origin by construction (a path, not a URL), and the cookie is what
    // the server authenticates; XHR sends it for a same-origin request.
    request.setRequestHeader('Accept', 'application/json');
    if (request.upload) {
      request.upload.addEventListener('progress', function (event) {
        if (!event.lengthComputable) return;
        var percent = Math.round((event.loaded / event.total) * 100);
        say('Uploading ' + plural(queue.length, 'file') + '… ' + percent + '%');
      });
    }
    request.addEventListener('load', function () {
      busy = false;
      if (drop) drop.classList.remove('ml-busy');
      var result = {};
      try {
        result = JSON.parse(request.responseText) || {};
      } catch (e) {
        result = {};
      }
      var stored = (result.stored || []).length;
      var refused = result.refused || [];
      if (stored) {
        // Reloading is what shows the new files: the grid is server-rendered,
        // sorted and paged, and a tile inserted by hand would be in the wrong
        // place on every ordering but one.
        //
        // To `pathname`, dropping the query: the filters in force are quite
        // likely not the ones the new file matches — upload a PDF while the
        // Images tab is open and a reload in place would appear to have done
        // nothing. The unfiltered first page always shows it.
        say('Uploaded ' + plural(stored, 'file') + '. Refreshing…');
        location.href = location.pathname;
        return;
      }
      var problems = refused.concat(tooBig.map(function (name) { return name + ': too large'; }));
      say(problems.length ? problems.join('; ') : 'Nothing was uploaded.');
    });
    request.addEventListener('error', function () {
      busy = false;
      if (drop) drop.classList.remove('ml-busy');
      say('The upload could not be sent. Check your connection and try again.');
    });
    say('Uploading ' + plural(queue.length, 'file') + '…');
    request.send(data);
  }

  if (input) {
    input.addEventListener('change', function () {
      send(input.files);
      // Cleared so choosing the same file twice in a row fires `change` again.
      input.value = '';
    });
  }

  if (drop) {
    // The hint ships hidden and is revealed here, because without this script
    // dropping a file on the page does nothing — an instruction that is only
    // true once the script has run belongs with the script that makes it true.
    var hint = drop.querySelector('[data-ml-hint]');
    if (hint) hint.hidden = false;

    // `dragover` has to be prevented on every event, not just on entry, or
    // the browser treats the page as a non-target and opens the file instead
    // of dropping it.
    ['dragenter', 'dragover'].forEach(function (name) {
      drop.addEventListener(name, function (event) {
        if (!event.dataTransfer || Array.prototype.indexOf.call(event.dataTransfer.types || [], 'Files') === -1) return;
        event.preventDefault();
        drop.classList.add('ml-over');
      });
    });
    ['dragleave', 'dragend'].forEach(function (name) {
      drop.addEventListener(name, function (event) {
        // `dragleave` also fires when the pointer crosses into a child, which
        // would flicker the highlight off and on over every tile.
        if (event.target !== drop && drop.contains(event.target)) return;
        drop.classList.remove('ml-over');
      });
    });
    drop.addEventListener('drop', function (event) {
      if (!event.dataTransfer) return;
      event.preventDefault();
      drop.classList.remove('ml-over');
      send(event.dataTransfer.files);
    });
  }

  // A file name is long and a tile is narrow, so the name is ellipsed in CSS.
  // The title attribute is what gives the whole of it back on hover; set from
  // the text rather than written into every tile server-side.
  document.querySelectorAll('.ml-tile .ml-name').forEach(function (name) {
    if (!name.title) name.title = name.textContent;
  });
})();
