/* Shared Koenig services for post/page and automation email editors. */
(function () {
  'use strict';
  window.GhostKoenig = {create: function (config, notify) {
    var React = window.React;
    var base = config.basePath || '';
    var banner = notify || function (message) { console.error(message); };
  /* ---------------------------------------------------------------- upload */

  // Matches admin-x-framework's koenigFileUploadTypes. Only the endpoints
  // this server actually implements are listed; anything else would fail at
  // upload time with a confusing error rather than an honest one.
  var FILE_TYPES = {
    image: {
      mimeTypes: ['image/gif', 'image/jpg', 'image/jpeg', 'image/png', 'image/svg+xml', 'image/webp', 'image/avif'],
      extensions: ['gif', 'jpg', 'jpeg', 'png', 'svg', 'svgz', 'webp', 'avif'],
      endpoint: '/ghost/api/admin/images/upload/'
    },
    mediaThumbnail: {
      mimeTypes: ['image/gif', 'image/jpg', 'image/jpeg', 'image/png', 'image/webp'],
      extensions: ['gif', 'jpg', 'jpeg', 'png', 'webp'],
      endpoint: '/ghost/api/admin/images/upload/'
    },
    video: {
      mimeTypes: ['video/mp4', 'video/webm', 'video/ogg'],
      extensions: ['mp4', 'webm', 'ogv'],
      endpoint: '/ghost/api/admin/media/upload/'
    },
    audio: {
      mimeTypes: ['audio/mp3', 'audio/mpeg', 'audio/ogg', 'audio/wav', 'audio/mp4', 'audio/x-m4a'],
      extensions: ['mp3', 'wav', 'ogg', 'm4a'],
      endpoint: '/ghost/api/admin/media/upload/'
    },
    file: {
      mimeTypes: [],
      extensions: [],
      endpoint: '/ghost/api/admin/files/upload/'
    }
  };

  // The upload endpoints answer either Ghost's `{images: [{url}]}` envelope
  // or this server's bare `[{url}]`, so both shapes are unwrapped.
  function urlFromResponse(body) {
    var list = body;
    if (body && !Array.isArray(body)) {
      list = body.images || body.media || body.files || body.data;
    }
    if (Array.isArray(list) && list[0] && list[0].url) { return list[0].url; }
    return undefined;
  }

  function uploadOne(file, type, onProgress) {
    return new Promise(function (resolve, reject) {
      var data = new FormData();
      data.append('file', file, file.name);
      var xhr = new XMLHttpRequest();
      xhr.open('POST', base + FILE_TYPES[type].endpoint, true);
      xhr.upload.onprogress = function (e) {
        if (e.lengthComputable) { onProgress(Math.round((e.loaded / e.total) * 100)); }
      };
      xhr.onload = function () {
        var body = null;
        try { body = JSON.parse(xhr.responseText); } catch (err) { body = null; }
        if (xhr.status >= 200 && xhr.status < 300) {
          var url = urlFromResponse(body);
          if (!url) { reject({message: 'The upload returned no file URL.', fileName: file.name}); return; }
          resolve({url: url, fileName: file.name});
        } else {
          var message = (body && body.errors && body.errors[0] && body.errors[0].message) || ('Upload failed (' + xhr.status + ')');
          reject({message: message, fileName: file.name});
        }
      };
      xhr.onerror = function () { reject({message: 'Upload failed', fileName: file.name}); };
      xhr.send(data);
    });
  }

  // Koenig's cards call this hook and read {upload, progress, isLoading,
  // errors, filesNumber} off it.
  function useFileUpload(type) {
    type = type || 'image';
    var state = React.useState(0);
    var progress = state[0], setProgress = state[1];
    var loading = React.useState(false);
    var isLoading = loading[0], setLoading = loading[1];
    var errored = React.useState([]);
    var errors = errored[0], setErrors = errored[1];
    var counted = React.useState(0);
    var filesNumber = counted[0], setFilesNumber = counted[1];

    var upload = React.useCallback(function (files) {
      files = Array.prototype.slice.call(files || []);
      setFilesNumber(files.length);
      setLoading(true);
      setProgress(0);

      var allowed = FILE_TYPES[type].extensions;
      var rejected = [];
      if (allowed && allowed.length) {
        files.forEach(function (file) {
          var ext = (file.name.split('.').pop() || '').toLowerCase();
          if (allowed.indexOf(ext) === -1) {
            rejected.push({fileName: file.name, message: 'Unsupported file type. Use .' + allowed.join(', .').toUpperCase()});
          }
        });
      }
      if (rejected.length) {
        setErrors(rejected);
        setLoading(false);
        setProgress(100);
        return Promise.resolve(null);
      }

      var done = 0;
      return Promise.all(files.map(function (file) {
        return uploadOne(file, type, function (pct) {
          setProgress(Math.round((done * 100 + pct) / files.length));
        }).then(function (result) {
          done += 1;
          setProgress(Math.round((done / files.length) * 100));
          return result;
        });
      })).then(function (results) {
        setLoading(false);
        setErrors([]);
        return results;
      }).catch(function (err) {
        setErrors(function (previous) { return previous.concat([err]); });
        setLoading(false);
        setProgress(100);
        return null;
      });
    }, [type]);

    return {progress: progress, isLoading: isLoading, upload: upload, errors: errors, filesNumber: filesNumber};
  }

  async function snippetRequest(method, path, data) {
    var response = await fetch(base + '/ghost/api/admin/snippets/' + (path || '') + '?limit=all&formats=mobiledoc,lexical', {
      method: method,
      credentials: 'same-origin',
      headers: {'Content-Type': 'application/json'},
      body: data ? JSON.stringify({snippets: [data]}) : undefined
    });
    if (response.status === 204) return null;
    var result = await response.json().catch(function () { return {}; });
    if (!response.ok) throw new Error(result.errors?.[0]?.message || 'Could not save or load snippets.');
    return result;
  }

  function editorSnippet(row) {
    try {
      var value = JSON.parse(row.lexical);
      if (typeof value === 'string') value = JSON.parse(value);
      if (!Array.isArray(value.nodes) || !value.nodes.length) return null;
      return {id: row.id, name: row.name, value: JSON.stringify(value)};
    } catch (error) { return null; }
  }

  function confirmSnippet(title, message, action, danger) {
    return new Promise(function (resolve) {
      var dialog = document.createElement('dialog');
      var heading = document.createElement('h3');
      heading.id = 'snippet-confirm-title';
      heading.textContent = title;
      dialog.setAttribute('aria-labelledby', heading.id);
      var description = document.createElement('p');
      description.textContent = message;
      var actions = document.createElement('form');
      actions.method = 'dialog';
      actions.className = 'd-actions';
      var cancel = document.createElement('button');
      cancel.className = 'btn text';
      cancel.value = 'cancel';
      cancel.textContent = 'Cancel';
      var accept = document.createElement('button');
      accept.className = 'btn' + (danger ? ' danger' : '');
      accept.value = 'confirm';
      accept.textContent = action;
      actions.append(cancel, accept);
      dialog.append(heading, description, actions);
      dialog.addEventListener('close', function () {
        resolve(dialog.returnValue === 'confirm');
        dialog.remove();
      }, {once: true});
      document.body.appendChild(dialog);
      dialog.showModal();
      cancel.focus();
    });
  }

  // The bookmark and embed cards need the server to fetch the target's
  // oEmbed/metadata; without that endpoint they fall back to a plain link.
  function fetchEmbed(url, options) {
    var query = '?url=' + encodeURIComponent(url) + (options && options.type ? '&type=' + options.type : '');
    return fetch(base + '/ghost/api/admin/oembed/' + query, {headers: {'Content-Type': 'application/json'}})
      .then(function (response) {
        if (!response.ok) { throw new Error('Could not fetch ' + url); }
        return response.json();
      });
  }

  function useCardConfig() {
    var snippetState = React.useState([]);
    var snippets = snippetState[0], setSnippets = snippetState[1];
    var manageState = React.useState(false);
    var canManageSnippets = manageState[0], setCanManageSnippets = manageState[1];
    var snippetBusy = React.useRef(false);
    React.useEffect(function () {
      var mounted = true;
      snippetRequest('GET').then(function (result) {
        if (!mounted) return;
        setSnippets((result.snippets || []).map(editorSnippet).filter(Boolean));
        setCanManageSnippets(!!result.meta?.can_manage);
      }).catch(function (error) { if (mounted) banner('Snippets could not be loaded. ' + error.message, 'error'); });
      return function () { mounted = false; };
    }, []);

    async function createSnippet(value) {
      if (snippetBusy.current) return;
      snippetBusy.current = true;
      try {
        var name = value.name.trim();
        var existing = snippets.find(function (snippet) { return snippet.name.toLowerCase() === name.toLowerCase(); });
        if (existing && !await confirmSnippet('Update this snippet?', '“' + existing.name + '” will be replaced for future use. Existing posts will stay the same.', 'Update snippet', false)) return;
        var result = await snippetRequest(existing ? 'PUT' : 'POST', existing ? existing.id + '/' : '', {
          name: existing ? existing.name : name, lexical: value.value, mobiledoc: '{}'
        });
        var saved = editorSnippet(result.snippets?.[0] || {});
        if (!saved) throw new Error('The saved snippet could not be loaded.');
        setSnippets(function (current) {
          return current.filter(function (snippet) { return snippet.id !== saved.id; }).concat(saved).sort(function (a, b) { return a.name.localeCompare(b.name); });
        });
        banner('Snippet “' + saved.name + '” ' + (existing ? 'updated.' : 'saved.'), 'ok');
      } catch (error) { banner(error.message || 'Snippet save failed.', 'error'); }
      finally { snippetBusy.current = false; }
    }

    async function deleteSnippet(value) {
      if (snippetBusy.current) return;
      snippetBusy.current = true;
      try {
        if (!await confirmSnippet('Delete snippet?', '“' + value.name + '” will be removed for all staff. Existing posts will stay the same.', 'Delete snippet', true)) return;
        await snippetRequest('DELETE', value.id + '/');
        setSnippets(function (current) { return current.filter(function (snippet) { return snippet.id !== value.id; }); });
        banner('Snippet deleted.', 'ok');
      } catch (error) { banner(error.message || 'Snippet delete failed.', 'error'); }
      finally { snippetBusy.current = false; }
    }

    return {
            // Cards ask the host what the site can do; answering honestly
            // keeps menu items that would not work from being offered.
            siteTitle: config.siteTitle || '',
            siteDescription: config.siteDescription || '',
            siteUrl: config.siteUrl || '',
            membersEnabled: !!config.membersEnabled,
            stripeEnabled: !!config.stripeEnabled,
            feature: {},
            snippets: snippets,
            createSnippet: canManageSnippets ? createSnippet : undefined,
            deleteSnippet: canManageSnippets ? deleteSnippet : undefined,
            deprecated: {headerV1: true},
            // Koenig builds its Unsplash provider straight from this object,
            // using it as the request headers against api.unsplash.com. Null
            // falls back to its own bundled sample gallery, which is what an
            // install with the app on but no access key gets.
            unsplash: config.unsplash || null,
            pinturaConfig: config.pinturaConfig || null,
            fetchEmbed: fetchEmbed,
            fetchAutocompleteLinks: function (query) {
              var text = typeof query === 'string' ? query : query && (query.search || query.query) || '';
              return fetch(base + '/ghost/api/admin/editor/links/?q=' + encodeURIComponent(text), {credentials:'same-origin'})
                .then(function(response){if(!response.ok)throw new Error('Link search failed');return response.json();})
                .then(function(data){return [{label:'Homepage',value:config.siteUrl+'/'}].concat(data.links||[]);});
            },
            searchLinks: function (query) {
              var text = typeof query === 'string' ? query : query && (query.search || query.query) || '';
              return fetch(base + '/ghost/api/admin/editor/links/?q=' + encodeURIComponent(text), {credentials:'same-origin'})
                .then(function(response){if(!response.ok)throw new Error('Link search failed');return response.json();})
                // Koenig's search UI expects groups with `items`; the simple
                // settings autocomplete above expects a flat list instead.
                .then(function(data){return data.links && data.links.length ? [{label:'Posts and pages',items:data.links}] : [];});
            },
            renderLabels: false,
            fetchLabels: function () { return Promise.resolve([]); }
          };
  }
  return {fileUploader: {useFileUpload: useFileUpload, fileTypes: FILE_TYPES}, useCardConfig: useCardConfig};
  }};
})();
