/* The form builder: add, edit, reorder and remove a form's fields.
 *
 * The page hands over the form's current fields and the field types on offer
 * in `data-builder`. This script draws one card per field and, on save,
 * writes the whole list into the hidden `fields` input as JSON.
 *
 * It is a convenience, not an authority: the server rebuilds the definition
 * from that JSON and re-checks every part of it (ghost_core::forms::
 * normalize_fields), so nothing done to this page can store a field the
 * server would not have accepted. Text is placed with textContent and
 * .value only — a label containing markup is shown as the characters typed.
 */
(function () {
  'use strict';
  var form = document.getElementById('form-builder');
  var list = document.getElementById('fb-list');
  if (!form || !list) { return; }

  var config = {};
  try { config = JSON.parse(form.dataset.builder || '{}'); } catch (err) { config = {}; }
  var kinds = config.kinds || [];
  var maxFields = config.maxFields || 30;
  var fields = (config.fields || []).map(function (field) {
    return {
      key: field.key || '',
      kind: field.kind || 'text',
      label: field.label || '',
      required: !!field.required,
      placeholder: field.placeholder || '',
      help: field.help || '',
      options: (field.options || []).join('\n')
    };
  });

  var ICONS = {
    up: 'M7.41 15.41L12 10.83l4.59 4.58L18 14l-6-6-6 6z',
    down: 'M7.41 8.59L12 13.17l4.59-4.58L18 10l-6 6-6-6z',
    remove: 'M6 19c0 1.1.9 2 2 2h8c1.1 0 2-.9 2-2V7H6v12zM19 4h-3.5l-1-1h-5l-1 1H5v2h14V4z'
  };

  function kindOf(value) {
    for (var i = 0; i < kinds.length; i++) { if (kinds[i].value === value) { return kinds[i]; } }
    return {value: value, label: value, options: false, placeholder: true};
  }

  function el(tag, className, text) {
    var node = document.createElement(tag);
    if (className) { node.className = className; }
    if (text !== undefined) { node.textContent = text; }
    return node;
  }

  function icon(name) {
    var svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
    svg.setAttribute('class', 'i');
    svg.setAttribute('viewBox', '0 0 24 24');
    svg.setAttribute('fill', 'currentColor');
    svg.setAttribute('aria-hidden', 'true');
    var path = document.createElementNS('http://www.w3.org/2000/svg', 'path');
    path.setAttribute('d', ICONS[name]);
    svg.appendChild(path);
    return svg;
  }

  function tool(name, title, disabled, onClick) {
    var button = el('button', 'icon-btn' + (name === 'remove' ? ' danger' : ''));
    button.type = 'button';
    button.title = title;
    button.setAttribute('aria-label', title);
    button.disabled = !!disabled;
    button.appendChild(icon(name));
    button.addEventListener('click', onClick);
    return button;
  }

  /* A floating-label text input in the admin's own style. */
  function input(labelText, value, maxLength, onInput, multiline) {
    var label = el('label', 'field');
    var control = document.createElement(multiline ? 'textarea' : 'input');
    control.placeholder = ' ';
    control.value = value;
    control.maxLength = maxLength;
    if (multiline) { control.rows = 4; }
    control.addEventListener('input', function () { onInput(control.value); });
    label.appendChild(control);
    label.appendChild(el('span', '', labelText));
    return label;
  }

  function fieldCard(field, index) {
    var kind = kindOf(field.kind);
    var card = el('div', 'fb-field');

    var head = el('div', 'fb-head');
    head.appendChild(el('span', 'fb-num', String(index + 1)));
    head.appendChild(el('span', 'fb-kind', kind.label));
    var tools = el('div', 'fb-tools');
    tools.appendChild(tool('up', 'Move up', index === 0, function () { move(index, -1); }));
    tools.appendChild(tool('down', 'Move down', index === fields.length - 1, function () { move(index, 1); }));
    tools.appendChild(tool('remove', 'Remove field', false, function () {
      fields.splice(index, 1);
      render();
    }));
    head.appendChild(tools);
    card.appendChild(head);

    var grid = el('div', 'fb-grid');
    var label = input(field.kind === 'consent' ? 'Checkbox text' : 'Label', field.label, 120, function (value) { field.label = value; });
    label.querySelector('input').required = true;
    if (!kind.placeholder) { label.classList.add('wide'); }
    grid.appendChild(label);
    if (kind.placeholder) {
      grid.appendChild(input('Placeholder · optional', field.placeholder, 120, function (value) { field.placeholder = value; }));
    }
    if (kind.options) {
      var options = input('Options · one per line', field.options, 8000, function (value) { field.options = value; }, true);
      options.classList.add('wide');
      grid.appendChild(options);
    }
    var help = input('Help text · optional', field.help, 300, function (value) { field.help = value; });
    help.classList.add('wide');
    grid.appendChild(help);
    card.appendChild(grid);

    var required = el('label', 'fb-required');
    var box = el('span', 'switch');
    var check = document.createElement('input');
    check.type = 'checkbox';
    check.checked = field.required;
    check.addEventListener('change', function () { field.required = check.checked; });
    var track = el('span', 'track');
    track.appendChild(el('span', 'thumb'));
    box.appendChild(check);
    box.appendChild(track);
    required.appendChild(box);
    required.appendChild(el('span', '', field.kind === 'consent' ? 'Must be ticked to send' : 'Required'));
    card.appendChild(required);
    return card;
  }

  function move(index, by) {
    var target = index + by;
    if (target < 0 || target >= fields.length) { return; }
    var moved = fields.splice(index, 1)[0];
    fields.splice(target, 0, moved);
    render();
  }

  function render() {
    list.textContent = '';
    if (!fields.length) {
      list.appendChild(el('div', 'fb-empty', 'This form has no fields. Add one below.'));
    }
    fields.forEach(function (field, index) { list.appendChild(fieldCard(field, index)); });
    var count = document.getElementById('fb-count');
    if (count) { count.textContent = fields.length + ' of ' + maxFields + ' fields'; }
    var add = document.getElementById('fb-add');
    if (add) { add.disabled = fields.length >= maxFields; }
  }

  var picker = document.getElementById('fb-add-kind');
  if (picker) {
    kinds.forEach(function (kind) {
      var option = el('option', '', kind.label);
      option.value = kind.value;
      picker.appendChild(option);
    });
  }
  var addButton = document.getElementById('fb-add');
  if (addButton) {
    addButton.addEventListener('click', function () {
      if (fields.length >= maxFields) { return; }
      var kind = kindOf(picker ? picker.value : 'text');
      // No key: the server issues one when the form is saved.
      fields.push({key: '', kind: kind.value, label: '', required: false, placeholder: '', help: '', options: ''});
      render();
      var cards = list.querySelectorAll('.fb-field');
      var last = cards[cards.length - 1];
      if (last) {
        last.scrollIntoView({block: 'center', behavior: 'smooth'});
        var first = last.querySelector('input');
        if (first) { first.focus({preventScroll: true}); }
      }
    });
  }

  /* ---- what happens after a response ---- */

  var action = document.getElementById('fb-action');
  function paintAction() {
    if (!action) { return; }
    Array.prototype.forEach.call(document.querySelectorAll('[data-fb-for]'), function (node) {
      node.hidden = node.getAttribute('data-fb-for').split(' ').indexOf(action.value) === -1;
    });
  }
  if (action) { action.addEventListener('change', paintAction); paintAction(); }

  // Upload a file to link to. It goes through the admin's own upload
  // endpoint — the same one the editor's file card uses — and the address
  // that comes back is put in the link field; saving the form is still a
  // separate, deliberate step.
  var uploadButton = document.getElementById('fb-upload');
  var fileInput = document.getElementById('fb-file');
  var urlInput = document.getElementById('fb-url');
  var uploadState = document.getElementById('fb-upload-state');
  function say(text) { if (uploadState) { uploadState.textContent = text; } }
  function uploadedUrl(body) {
    var list = body;
    if (body && !Array.isArray(body)) { list = body.files || body.images || body.media || body.data; }
    return Array.isArray(list) && list[0] && typeof list[0].url === 'string' ? list[0].url : '';
  }
  if (uploadButton && fileInput && urlInput && config.uploadUrl) {
    uploadButton.addEventListener('click', function () { fileInput.click(); });
    fileInput.addEventListener('change', function () {
      var file = fileInput.files && fileInput.files[0];
      if (!file) { return; }
      var data = new FormData();
      data.append('file', file, file.name);
      uploadButton.disabled = true;
      say('Uploading ' + file.name + '…');
      fetch(config.uploadUrl, {method: 'POST', body: data, credentials: 'same-origin'}).then(function (response) {
        return response.json().catch(function () { return null; }).then(function (body) {
          var url = response.ok ? uploadedUrl(body) : '';
          if (!url) {
            var reason = body && body.errors && body.errors[0] && body.errors[0].message;
            throw new Error(reason || 'The upload failed (' + response.status + ').');
          }
          return url;
        });
      }).then(function (url) {
        // Stored as a path on this site when it is one, so the link keeps
        // working if the site's address changes.
        try {
          var parsed = new URL(url, window.location.href);
          if (parsed.origin === window.location.origin) {
            url = parsed.pathname;
            // The link is stored without the install's base path, which is
            // added back when it is used; left on, it would be added twice.
            var basePath = config.basePath || '';
            if (basePath && url.indexOf(basePath + '/') === 0) { url = url.slice(basePath.length); }
          }
        } catch (err) { /* keep what the server gave */ }
        urlInput.value = url;
        say('Uploaded ' + file.name + '. Save the form to use it.');
      }).catch(function (err) {
        say((err && err.message) || 'The upload failed.');
      }).then(function () {
        uploadButton.disabled = false;
        fileInput.value = '';
      });
    });
  }

  form.addEventListener('submit', function () {
    var payload = fields.map(function (field) {
      return {
        key: field.key,
        kind: field.kind,
        label: field.label,
        required: field.required,
        placeholder: field.placeholder,
        help: field.help,
        options: field.options.split('\n')
      };
    });
    document.getElementById('fb-fields-json').value = JSON.stringify(payload);
  });

  render();
})();
