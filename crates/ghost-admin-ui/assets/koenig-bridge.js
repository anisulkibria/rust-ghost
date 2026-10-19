/* Mounts Ghost's Koenig editor into the admin's post/page form.
 *
 * koenig-lexical ships as a UMD bundle that expects React on the window and
 * exposes itself as window['@tryghost/koenig-lexical']. It is a React
 * component tree, not a widget, so this file is the whole integration: it
 * renders <KoenigComposer><KoenigEditor/></KoenigComposer>, feeds it the
 * post's saved Lexical state, and on submit writes the editor's state and
 * rendered HTML into hidden inputs for the server to store.
 *
 * The HTML is produced here rather than on the server because the cards know
 * how to render themselves: every Koenig node implements exportDOM, which is
 * exactly what Ghost's own server-side renderer calls. convertToHtml() below
 * mirrors @tryghost/kg-lexical-html-renderer's $convertToHtmlString so the
 * markup matches what Ghost would have written.
 */
(function () {
  var mount = document.getElementById('koenig-mount');
  if (!mount) { return; }

  var Koenig = window['@tryghost/koenig-lexical'];
  var React = window.React;
  var ReactDOM = window.ReactDOM;
  if (!Koenig || !React || !ReactDOM) {
    showFallback('The editor failed to load. Edit the Markdown below instead.');
    return;
  }

  var form = document.getElementById('editor-form');
  var lexicalInput = document.getElementById('field-lexical');
  var htmlInput = document.getElementById('field-html');
  var config = JSON.parse(mount.dataset.config || '{}');
  var api = null;

  // The install's base path ('' at a domain root, '/blog' under a
  // subdirectory). Every endpoint below is written in site terms and prefixed
  // with this, because a reverse proxy fronting a subdirectory install only
  // forwards paths under it — an unprefixed '/ghost/api/...' would be
  // somebody else's URL.
  var base = config.basePath || '';

  /* ---------------------------------------------------------------- upload */

  // Matches admin-x-framework's koenigFileUploadTypes. Only the endpoints
  // this server actually implements are listed; anything else would fail at
  // upload time with a confusing error rather than an honest one.
  var FILE_TYPES = {
    image: {
      mimeTypes: ['image/gif', 'image/jpg', 'image/jpeg', 'image/png', 'image/svg+xml', 'image/webp'],
      extensions: ['gif', 'jpg', 'jpeg', 'png', 'svg', 'svgz', 'webp'],
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
          resolve({url: urlFromResponse(body), fileName: file.name});
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

  /* ----------------------------------------------- lexical state -> html */

  var FORMAT_TAGS = {1: 'STRONG', 2: 'EM', 4: 'S', 8: 'U', 16: 'CODE', 32: 'SUB', 64: 'SUP', 128: 'MARK'};
  // Low bit first, so tags nest in the order Ghost's renderer opens them.
  var FORMAT_BITS = [1, 2, 4, 8, 16, 32, 64, 128];

  function nodeType(node) { return node.getType ? node.getType() : ''; }
  function isElement(node) { return typeof node.getChildren === 'function'; }
  function isText(node) { return !isElement(node) && typeof node.getFormat === 'function' && typeof node.getTextContent === 'function' && nodeType(node) !== 'linebreak'; }
  function isLineBreak(node) { return nodeType(node) === 'linebreak'; }
  function isLink(node) { return nodeType(node) === 'link' || nodeType(node) === 'extended-link'; }
  // DecoratorNode is what every Koenig card extends, and decorate() is the
  // method that makes one. Checking for it avoids having to know the name of
  // every card the bundle ships.
  function isCard(node) { return typeof node.decorate === 'function'; }

  // Ghost 4+ heading anchors (kg-utils slugify).
  function slugify(text) {
    return encodeURIComponent(
      String(text || '').trim().toLowerCase()
        .replace(/[\][!"#$%&'()*+,./:;<=>?@\\^_{|}~]/g, '')
        .replace(/\s+/g, '-')
        .replace(/^-|-{2,}|-$/g, '')
    );
  }

  function headingId(text, used) {
    var id = slugify(text);
    if (used[id] !== undefined) {
      var deduped = id + '-' + used[id];
      used[id] += 1;
      return deduped;
    }
    used[id] = 1;
    return id;
  }

  // Renders a run of text/link/linebreak nodes, opening each format tag once
  // and keeping it open while the following nodes still carry that format.
  function renderTextRun(nodes) {
    var root = document.createElement('div');
    var current = root;
    var openFormats = [];

    for (var i = 0; i < nodes.length; i++) {
      var node = nodes[i];

      if (isLineBreak(node)) { current.append(document.createElement('BR')); continue; }

      if (isLink(node)) {
        var anchor = document.createElement('A');
        if (node.getURL && node.getURL()) { anchor.setAttribute('href', node.getURL()); }
        if (node.getRel && node.getRel()) { anchor.setAttribute('rel', node.getRel()); }
        anchor.innerHTML = exportChildren(node, {});
        current.append(anchor);
        continue;
      }

      if (!isText(node)) { continue; }

      var format = node.getFormat();
      if (format === 0) { current.append(node.getTextContent()); continue; }

      var remaining = nodes.slice(i + 1);
      var nextLink = remaining.findIndex(isLink);
      var sortable = (nextLink === -1 ? remaining : remaining.slice(0, nextLink)).filter(isText);

      var toOpen = FORMAT_BITS.filter(function (bit) {
        return (format & bit) !== 0 && openFormats.indexOf(bit) === -1;
      });
      // Formats that later nodes also use are opened first, so the tag that
      // has to live longest is the outermost one.
      toOpen.sort(function (a, b) {
        var ai = sortable.findIndex(function (n) { return (n.getFormat() & a) !== 0; });
        var bi = sortable.findIndex(function (n) { return (n.getFormat() & b) !== 0; });
        if (ai === -1) { return 1; }
        if (bi === -1) { return -1; }
        return ai - bi;
      });
      toOpen.forEach(function (bit) {
        var tag = document.createElement(FORMAT_TAGS[bit]);
        current.append(tag);
        current = tag;
        openFormats.push(bit);
      });

      current.append(node.getTextContent());

      var nextNode = remaining.find(function (n) { return isText(n) || isLink(n); });
      openFormats.slice().forEach(function (bit) {
        if (!nextNode || isLink(nextNode) || (isText(nextNode) && (nextNode.getFormat() & bit) === 0)) {
          current = current.parentNode;
          openFormats.pop();
        }
      });
    }

    return root.innerHTML;
  }

  function exportChildren(node, options) {
    var out = [];
    var run = [];
    var flush = function () {
      if (run.length) { out.push(renderTextRun(run)); run = []; }
    };
    node.getChildren().forEach(function (child) {
      if (isLineBreak(child) || isText(child) || isLink(child)) {
        run.push(child);
      } else {
        flush();
        if (isCard(child)) {
          out.push(exportTopLevel(child, options));
        } else if (isElement(child)) {
          out.push(exportChildren(child, options));
        }
      }
    });
    flush();
    return out.join('');
  }

  function exportList(node, options) {
    var tag = node.getTag();
    var start = node.getStart ? node.getStart() : 1;
    var out = [];
    var liOpen = false;

    node.getChildren().forEach(function (item) {
      if (nodeType(item) !== 'listitem') { return; }
      var inner = item.getChildren();
      if (inner[0] && nodeType(inner[0]) === 'list') {
        out.push(exportList(inner[0], options));
        if (liOpen) { out.push('</li>'); liOpen = false; }
      } else {
        if (liOpen) { out.push('</li>'); liOpen = false; }
        out.push('<li>' + exportChildren(item, options));
        liOpen = true;
      }
    });
    if (liOpen) { out.push('</li>'); }

    var content = out.join('');
    return (start && start !== 1)
      ? '<' + tag + ' start="' + start + '">' + content + '</' + tag + '>'
      : '<' + tag + '>' + content + '</' + tag + '>';
  }

  function exportTopLevel(node, options) {
    if (isCard(node)) {
      var result = node.exportDOM(options.editor, options);
      var element = result.element;
      if (!element) { return ''; }
      if (result.type === 'inner') { return element.innerHTML || ''; }
      if (result.type === 'value') { return typeof element.value === 'string' ? element.value : ''; }
      return element.outerHTML || '';
    }

    switch (nodeType(node)) {
      case 'paragraph':
        return '<p>' + exportChildren(node, options) + '</p>';
      case 'heading':
      case 'extended-heading':
        var tag = node.getTag();
        return '<' + tag + ' id="' + headingId(node.getTextContent(), options.usedIds) + '">' +
          exportChildren(node, options) + '</' + tag + '>';
      case 'list':
        return exportList(node, options);
      case 'quote':
      case 'extended-quote':
        return '<blockquote>' + exportChildren(node, options) + '</blockquote>';
      case 'aside':
        return '<blockquote class="kg-blockquote-alt">' + exportChildren(node, options) + '</blockquote>';
      default:
        return isElement(node) ? exportChildren(node, options) : '';
    }
  }

  function convertToHtml(editor) {
    var html = '';
    editor.getEditorState().read(function () {
      // $getRoot() is not exported by the UMD bundle, but every node can walk
      // up to the root it belongs to.
      var first = editor.getEditorState()._nodeMap.get('root');
      if (!first) { return; }
      var options = {
        editor: editor,
        target: 'html',
        // The card renderers build real elements; handing them this page's
        // document is the browser equivalent of the JSDOM Ghost passes.
        dom: {window: {document: document}},
        createDocument: function () { return document; },
        usedIds: {},
        usedIdAttributes: {},
        siteUrl: config.siteUrl || '',
        canTransformImage: function () { return false; }
      };
      var children = first.getChildren();
      var out = children.map(function (child) { return exportTopLevel(child, options); });
      // Koenig parks an empty paragraph at the end of every document.
      var last = children[children.length - 1];
      if (last && nodeType(last) === 'paragraph' && last.getTextContent().trim() === '') { out.pop(); }
      html = out.join('');
    });
    return html;
  }

  /* ------------------------------------------------------------- mounting */

  function showFallback(message) {
    var fallback = document.getElementById('markdown-fallback');
    if (!fallback) { return; }
    fallback.hidden = false;
    var note = fallback.querySelector('[data-fallback-note]');
    if (note && message) { note.textContent = message; }
    if (mount) { mount.hidden = true; }
  }

  var e = React.createElement;

  function Editor() {
    return e(
      'div',
      {className: 'koenig-react-editor koenig-lexical'},
      e(
        Koenig.KoenigComposer,
        {
          initialEditorState: config.lexical || undefined,
          fileUploader: {useFileUpload: useFileUpload, fileTypes: FILE_TYPES},
          cardConfig: {
            // Cards ask the host what the site can do; answering honestly
            // keeps menu items that would not work from being offered.
            siteTitle: config.siteTitle || '',
            siteDescription: config.siteDescription || '',
            siteUrl: config.siteUrl || '',
            membersEnabled: !!config.membersEnabled,
            stripeEnabled: !!config.stripeEnabled,
            feature: {},
            deprecated: {headerV1: true},
            unsplash: null,
            fetchEmbed: fetchEmbed,
            fetchAutocompleteLinks: function () {
              return Promise.resolve([{label: 'Homepage', value: config.siteUrl + '/'}]);
            },
            searchLinks: function () { return Promise.resolve([]); },
            renderLabels: false,
            fetchLabels: function () { return Promise.resolve([]); }
          },
          onError: function (error) { console.error(error); },
          darkMode: window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches,
          isTKEnabled: false
        },
        e(Koenig.KoenigEditor, {
          placeholderText: config.placeholder || 'Begin writing your post...',
          registerAPI: function (editorApi) { api = editorApi; },
          cursorDidExitAtTop: function () {
            var title = document.getElementById('field-title');
            if (title) { title.focus(); }
          }
        }),
        // The plugin hands over a plain count, debounced as you type.
        e(Koenig.WordCountPlugin, {
          onChange: function (words) {
            var label = document.getElementById('word-count');
            if (label) { label.textContent = words + (words === 1 ? ' word' : ' words'); }
          }
        })
      )
    );
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

  try {
    ReactDOM.createRoot(mount).render(e(Editor));
  } catch (error) {
    console.error(error);
    showFallback('The editor failed to load. Edit the Markdown below instead.');
    return;
  }

  /* --------------------------------------------------------------- saving */

  // The form posts normally; this only fills in the two fields the editor
  // owns. If serialising ever throws, the submit is stopped rather than
  // quietly saving an empty post over a real one.
  form.addEventListener('submit', function (event) {
    if (!api) { return; }
    try {
      lexicalInput.value = api.serialize();
      htmlInput.value = convertToHtml(api.editorInstance);
    } catch (error) {
      console.error(error);
      event.preventDefault();
      window.alert('The editor could not save its content. Copy your work before reloading.');
    }
  });

  // Ghost's own cmd/ctrl-S.
  document.addEventListener('keydown', function (event) {
    if ((event.metaKey || event.ctrlKey) && event.key === 's') {
      event.preventDefault();
      if (form.requestSubmit) { form.requestSubmit(); } else { form.submit(); }
    }
  });
})();
