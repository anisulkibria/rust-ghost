/* Mounts Ghost's Koenig editor into the admin's post/page form, and owns
 * everything about saving it.
 *
 * koenig-lexical ships as a UMD bundle that expects React on the window and
 * exposes itself as window['@tryghost/koenig-lexical']. It is a React
 * component tree, not a widget, so this file is the whole integration: it
 * renders <KoenigComposer><KoenigEditor/></KoenigComposer>, feeds it the
 * post's saved Lexical state, and writes the editor's state and rendered
 * HTML into hidden inputs for the server to store.
 *
 * Saving works the way Ghost's editor does rather than the way a form does:
 * body edits to a draft are autosaved a moment after the typing stops, a
 * settings field commits the moment it is changed, and the body of something
 * already published waits for an explicit save. Saves are fetches that
 * answer JSON, so the page never navigates away from the writing — the
 * first save of a new post creates it and rewrites the address bar in
 * place. Without this script the form still posts normally and the server
 * redirects, which is how the Markdown fallback and a no-JS browser save.
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
    // The page script disables the textarea so a stale copy of the content
    // cannot be posted alongside the editor's; owning it back is part of
    // showing it, or the fallback could be read but not written.
    var textarea = fallback.querySelector('textarea');
    if (textarea) { textarea.disabled = false; }
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
            // Koenig builds its Unsplash provider straight from this object,
            // using it as the request headers against api.unsplash.com. Null
            // falls back to its own bundled sample gallery, which is what an
            // install with the app on but no access key gets.
            unsplash: config.unsplash || null,
            pinturaConfig: config.pinturaConfig || null,
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
          // Every keystroke in the body. This is what starts the autosave
          // clock — the serialize() calls only happen when a save does.
          onChange: function () { contentEdited(); },
          cursorDidExitAtTop: function () {
            var title = document.getElementById('field-title');
            if (title) { title.focus(); }
          }
        }),
        // The plugin hands over a plain count, debounced as you type.
        e(Koenig.WordCountPlugin, {
          onChange: function (words) {
            wordCount = words;
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

  /* --------------------------------------------------------------- saving
   *
   * A draft's body autosaves three seconds after typing stops; the title's
   * blur and settings fields commit at once. Status changes wait for the
   * primary action, and background saves preserve the last stored status;
   * the body of something published, scheduled or sent is held until an
   * explicit save — Save or cmd/ctrl-S — while its settings still save the
   * moment they change. Unsaved work is said out loud, and leaving with work
   * in flight is warned about.
   */

  // Ghost's editor engine debounces body autosave by three seconds; nothing
  // in this admin overrides it.
  var DEBOUNCE_MS = 3000;

  var isNew = !!config.isNew;
  var dirty = false;
  var saving = false;
  var queued = null;
  var savedStatus = config.savedStatus || 'draft';
  var revision = 0;
  var timer = null;
  var bannerTimer = null;
  var toastTimer = null;
  // The count the WordCountPlugin last handed over, so a save can tell a post
  // with nothing in it from one whose title alone is worth keeping.
  var wordCount = 0;

  function setState(text, kind) {
    var el = document.getElementById('save-state');
    if (el) {
      el.textContent = text;
      el.className = 'save-state' + (kind ? ' ' + kind : '');
    }
  }

  function statusValue() {
    var sel = form.querySelector('select[name="status"]');
    return sel ? sel.value : 'draft';
  }

  // "Something already out": its body waits for an explicit save, while a
  // draft's does not.
  function isDraft() { return savedStatus === 'draft'; }

  function paintSaveButton() {
    var label = document.querySelector('[data-save-label]');
    if (label) {
      label.textContent = isDraft() && statusValue() === 'published' ? 'Publish' :
        'Save ' + (config.postType === 'page' ? 'page' : 'post');
    }
  }

  function iconSvg(path) {
    return '<svg class="i" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="' + path + '"/></svg>';
  }
  var ICON_CHECK = iconSvg('M9 16.17L4.83 12l-1.42 1.41L9 19 21 7l-1.41-1.41z');
  var ICON_FLAG = iconSvg('M14.4 6L14 4H5v17h2v-7h5.6l.4 2h7V6z');

  // A one-shot banner where the page's flash banners sit. It leaves on its
  // own, because most of what it reports ("Published.") is good news the
  // writer has already moved past.
  function banner(message, kind) {
    var host = document.querySelector('.content');
    if (!host) { return; }
    var old = document.getElementById('save-banner');
    if (old) { old.remove(); }
    var div = document.createElement('div');
    div.id = 'save-banner';
    div.className = 'banner ' + (kind === 'error' ? 'error' : 'ok') + ' rise';
    div.innerHTML = (kind === 'error' ? ICON_FLAG : ICON_CHECK) + '<span class="msg"></span>';
    div.querySelector('.msg').textContent = message;
    var head = host.querySelector('.page-head');
    host.insertBefore(div, head);
    clearTimeout(bannerTimer);
    bannerTimer = setTimeout(function () { div.remove(); }, 6000);
  }

  function publishedToast() {
    var toast = document.getElementById('publish-toast');
    if (!toast) {
      toast = document.createElement('div');
      toast.id = 'publish-toast';
      toast.setAttribute('role', 'status');
      toast.setAttribute('aria-live', 'polite');
      document.body.appendChild(toast);
    }
    toast.textContent = (config.postType === 'page' ? 'Page' : 'Post') + ' published successfully.';
    toast.classList.add('show');
    clearTimeout(toastTimer);
    toastTimer = setTimeout(function () { toast.classList.remove('show'); }, 5000);
  }

  function paintSlug() {
    var preview = document.getElementById('slug-preview');
    var slugInput = document.getElementById('field-slug');
    if (!preview || !slugInput) { return; }
    preview.textContent = '';
    var slug = slugInput.value.trim();
    if (slug) {
      preview.append((config.siteUrl || '') + '/');
      var bold = document.createElement('b');
      bold.textContent = slug;
      preview.append(bold, '/');
    }
  }

  function titleValue() {
    var title = document.getElementById('field-title');
    return title ? title.value.trim() : '';
  }

  // What the server's answer asks the page to become. The first answer to a
  // new post is the big one: the post now exists, so the form's action, the
  // address bar and the delete button all have to point at it, as though the
  // page had been opened there from the start.
  function adopt(json, kind) {
    if (!json || !json.id) { return; }

    var statusSel = form.querySelector('select[name="status"]');
    savedStatus = json.status || savedStatus;
    var view = document.getElementById('editor-view');
    if (view) {
      view.hidden = savedStatus !== 'published' || !json.url;
      if (json.url) { view.href = json.url; }
    }
    if (statusSel && json.status && kind === 'explicit' && !queued) {
      var offered = Array.prototype.some.call(statusSel.options, function (o) { return o.value === json.status; });
      if (offered) { statusSel.value = json.status; }
    }

    var slugInput = document.getElementById('field-slug');
    if (slugInput && !slugInput.value.trim() && json.slug) {
      slugInput.value = json.slug;
      paintSlug();
    }

    paintSaveButton();
    if (!isNew) { return; }
    isNew = false;

    var url = base + (config.postType === 'page' ? '/ghost/pages/' : '/ghost/posts/') + json.id + '/';
    form.setAttribute('action', url);
    try { history.replaceState(history.state, '', url); } catch (err) { /* the form's action still moved */ }

    var delSec = document.getElementById('new-delete');
    if (delSec) {
      delSec.hidden = false;
      var btn = delSec.querySelector('button');
      var delForm = document.createElement('form');
      delForm.method = 'post';
      delForm.action = url + 'delete/';
      delForm.dataset.confirm = 'Delete ' + config.postType + '|“' + (titleValue() || 'Untitled') +
        '” will be permanently deleted.';
      if (btn) {
        delSec.appendChild(delForm);
        delForm.appendChild(btn);
        btn.type = 'submit';
      }
    }
  }

  function saveNow(kind) {
    if (saving) {
      if (queued !== 'explicit') { queued = kind; }
      return;
    }
    // Ghost's publish button stays inert until there is something to publish;
    // a save of a brand-new post with no words in it — asked for or autosaved
    // — would only litter the list with an empty draft.
    if (isNew && !titleValue() && wordCount === 0) { return; }

    // Content rides along unless this is a settings-only commit on something
    // already out. Ghost holds a published post's body for its explicit save,
    // so a tag ticked while the body is half-edited must not carry the half:
    // the inputs are emptied and the server leaves the content columns be.
    var withContent = kind === 'explicit' || isDraft();
    if (api) {
      try {
        if (withContent) {
          lexicalInput.value = api.serialize();
          htmlInput.value = convertToHtml(api.editorInstance);
        } else {
          lexicalInput.value = '';
          htmlInput.value = '';
        }
      } catch (error) {
        console.error(error);
        banner('The editor could not save its content. Copy your work before reloading.', 'error');
        setState('Not saved', 'error');
        return;
      }
    }

    var publishing = kind === 'explicit' && savedStatus !== 'published';
    var savingRevision = revision;
    saving = true;
    setState('Saving…', 'saving');
    // The form's own posts are urlencoded — a native submit sends nothing
    // else — and the server parses exactly that. A FormData body would go
    // out as multipart, which the parser would read as a form with no fields
    // in it at all, so the fields are re-packed the way a submit packs them.
    var fields = new URLSearchParams();
    new FormData(form).forEach(function (value, key) {
      if (typeof value === 'string') { fields.append(key, value); }
    });
    // Publishing requires the primary action. Background saves keep unpublished work private.
    if (kind !== 'explicit') { fields.set('status', savedStatus); }
    fetch(form.getAttribute('action'), {
      method: 'POST',
      body: fields,
      headers: {Accept: 'application/json'},
      credentials: 'same-origin'
    }).then(function (response) {
      return response.text().then(function (text) {
        var json = null;
        try { json = JSON.parse(text); } catch (err) { json = null; }
        if (!response.ok || !json || !json.id) {
          throw new Error((json && json.message) || ('The save failed (' + response.status + ').'));
        }
        return json;
      });
    }).then(function (json) {
      saving = false;
      dirty = revision !== savingRevision;
      adopt(json, kind);
      setState(dirty ? '* Unsaved changes' : 'Saved', dirty ? 'unsaved' : 'saved');
      if (publishing && json.status === 'published' && json.level !== 'error') {
        publishedToast();
      } else if (json.message) {
        banner(json.message, json.level === 'error' ? 'error' : 'ok');
      }
      if (queued) { var nextKind = queued; queued = null; saveNow(nextKind); }
    }).catch(function (err) {
      saving = false;
      queued = null;
      dirty = true;
      setState('Not saved', 'error');
      banner((err && err.message) || 'The save failed.', 'error');
    });
  }

  // The body or the title changed. A draft's edits start the autosave clock;
  // anything already out keeps them until Save, and says so.
  function contentEdited() {
    revision++;
    dirty = true;
    clearTimeout(timer);
    if (isDraft()) {
      setState('', '');
      timer = setTimeout(function () { timer = null; saveNow('auto'); }, DEBOUNCE_MS);
    } else {
      setState('* Unsaved changes', 'unsaved');
    }
  }

  // The form posts normally without this script; with it, saving stays on the
  // page — the fetch writes the post and the header says what happened.
  form.addEventListener('submit', function (event) {
    event.preventDefault();
    clearTimeout(timer);
    timer = null;
    saveNow('explicit');
  });

  // Ghost's own cmd/ctrl-S.
  document.addEventListener('keydown', function (event) {
    if ((event.metaKey || event.ctrlKey) && event.key === 's') {
      event.preventDefault();
      clearTimeout(timer);
      timer = null;
      saveNow('explicit');
    }
  });

  // Leaving with work unwritten is the one way to lose writing here, so it
  // gets the browser's own warning rather than a custom one that could be
  // raced past.
  window.addEventListener('beforeunload', function (event) {
    if (dirty || saving || timer) {
      event.preventDefault();
      event.returnValue = '';
    }
  });

  paintSaveButton();
  var markdownField = form.querySelector('textarea[name="markdown"]');
  if (markdownField) { markdownField.addEventListener('input', contentEdited); }

  var title = document.getElementById('field-title');
  if (title) {
    // The title is content like the body: drafts autosave it, and its blur —
    // the moment the writer considers it done — commits at once, as Ghost's
    // field commits do.
    title.addEventListener('input', contentEdited);
    title.addEventListener('blur', function () {
      if (dirty && isDraft()) {
        clearTimeout(timer);
        timer = null;
        saveNow('auto');
      }
    });
  }

  // Settings commit when changed, except status: selecting a publishing
  // status prepares the next primary action without publishing automatically.
  var side = document.querySelector('.editor-side');
  if (side) {
    side.addEventListener('change', function (event) {
      var field = event.target;
      if (!field.name || field.type === 'file') { return; }
      revision++;
      dirty = true;
      if (field.name === 'status') {
        // Choosing a publishing status sets the next explicit action.
        paintSaveButton();
        setState('* Unsaved changes', 'unsaved');
      } else {
        saveNow('settings');
      }
    });
  }

  /* ------------------------------------------------- the featured image */

  function paintFeature() {
    var drop = document.getElementById('fimg-drop');
    var img = document.getElementById('fimg-img');
    var input = document.getElementById('field-feature_image');
    if (!drop) { return; }
    var url = (input && input.value.trim()) || '';
    if (url && img) { img.src = url; }
    drop.classList.toggle('is-set', !!url);
  }

  // Ghost commits the feature image the moment it is chosen, on any status —
  // an image already on a published post is not half-finished writing.
  function setFeature(url, caption) {
    var input = document.getElementById('field-feature_image');
    if (input) { input.value = url || ''; }
    if (caption) {
      var cap = document.getElementById('field-feature_image_caption');
      if (cap) { cap.value = caption; }
    }
    paintFeature();
    revision++;
    dirty = true;
    saveNow('settings');
  }

  function wireFeatureImage() {
    var drop = document.getElementById('fimg-drop');
    if (!drop) { return; }
    var fileInput = document.getElementById('fimg-file');
    var prog = document.getElementById('fimg-prog');
    var bar = prog ? prog.querySelector('i') : null;

    var picker = function () { fileInput.click(); };
    var uploadBtn = document.getElementById('fimg-upload');
    var changeBtn = document.getElementById('fimg-change');
    if (uploadBtn) { uploadBtn.addEventListener('click', picker); }
    if (changeBtn) { changeBtn.addEventListener('click', picker); }

    var removeBtn = document.getElementById('fimg-remove');
    if (removeBtn) { removeBtn.addEventListener('click', function () { setFeature(''); }); }

    var urlField = document.getElementById('field-feature_image');
    if (urlField) { urlField.addEventListener('input', paintFeature); }

    if (fileInput) {
      fileInput.addEventListener('change', function () {
        var file = fileInput.files && fileInput.files[0];
        if (!file) { return; }
        if (prog) { prog.classList.add('on'); }
        if (bar) { bar.style.width = '0%'; }
        uploadOne(file, 'image', function (pct) {
          if (bar) { bar.style.width = pct + '%'; }
        }).then(function (result) {
          if (prog) { prog.classList.remove('on'); }
          fileInput.value = '';
          if (result && result.url) {
            setFeature(result.url);
          } else {
            banner('The upload came back without an image.', 'error');
          }
        }).catch(function (err) {
          if (prog) { prog.classList.remove('on'); }
          fileInput.value = '';
          banner((err && err.message) || 'The upload failed.', 'error');
        });
      });
    }

    var unsplashBtn = document.getElementById('fimg-unsplash');
    if (unsplashBtn) {
      unsplashBtn.addEventListener('click', uOpen);
    }
    paintFeature();
  }

  /* ------------------------------------------------- the unsplash picker
   *
   * A vanilla twin of the picker Ghost's editor opens (kg-unsplash's
   * UnsplashSearchModal), down to its behaviour: the latest photos are
   * already on the canvas when it opens, the search box searches live
   * (debounced, from the third character) and clears back to the latest
   * feed, scrolling to the bottom brings the next page from the Link
   * header, a tile hovers up the photographer's credit with a like count
   * and a download, clicking one zooms it, and Insert writes the same
   * URL and credit caption Ghost's does.
   */

  var uModal = null;
  var uNext = null;      // the Link header's rel="next", when there is one
  var uLoading = false;
  var uZoomed = null;
  var uSeq = 0;          // a search in flight answers to its own number
  var uController = null;
  var uSearchTimer = null;

  // Unsplash's guidelines ask for this referral on every link back to them.
  function uReferral(href) {
    try {
      var url = new URL(href);
      url.searchParams.set('utm_source', 'ghost');
      url.searchParams.set('utm_medium', 'referral');
      url.searchParams.set('utm_campaign', 'api-credit');
      return url.href;
    } catch (err) {
      return href;
    }
  }

  function uEsc(text) {
    return String(text == null ? '' : text).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
  }

  var U_HEART = iconSvg('M12 21.35l-1.45-1.32C5.4 15.36 2 12.28 2 8.5 2 5.42 4.42 3 7.5 3c1.74 0 3.41.81 4.5 2.09C13.09 3.81 14.76 3 16.5 3 19.58 3 22 5.42 22 8.5c0 3.78-3.4 6.86-8.55 11.54L12 21.35z');
  var U_DOWNLOAD = iconSvg('M19 9h-4V3H9v6H5l7 7 7-7zM5 18v2h14v-2H5z');

  function unsplashModal() {
    if (uModal) { return uModal; }
    uModal = document.createElement('dialog');
    uModal.className = 'unsplash';
    uModal.innerHTML =
      '<button class="u-close" type="button" aria-label="Close">' + iconSvg('M19 6.41L17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z') + '</button>' +
      '<header class="u-head"><h3>Unsplash</h3>' +
      '<div class="u-searchbox">' + iconSvg('M15.5 14h-.79l-.28-.27C15.41 12.59 16 11.11 16 9.5 16 5.91 13.09 3 9.5 3S3 5.91 3 9.5 5.91 16 9.5 16c1.61 0 3.09-.59 4.23-1.57l.27.28v.79l5 4.99L20.49 19l-4.99-5zm-6 0C7.01 14 5 11.99 5 9.5S7.01 5 9.5 5 14 7.01 14 9.5 11.99 14 9.5 14z') +
      '<input type="search" placeholder="Search free high-resolution photos" aria-label="Search Unsplash"></div></header>' +
      '<div class="u-body"><div class="u-grid"></div><div class="u-zoom" hidden></div>' +
      '<div class="u-loading" hidden><i></i></div></div>';
    document.body.appendChild(uModal);

    var input = uModal.querySelector('input');
    // Ghost's box searches live, 300ms after the last keystroke, from the
    // third character; emptying it goes back to the latest feed.
    input.addEventListener('input', function () {
      clearTimeout(uSearchTimer);
      uSearchTimer = setTimeout(function () {
        if (input.value.trim().length > 2) {
          uLoad('https://api.unsplash.com/search/photos?per_page=30&query=' + encodeURIComponent(input.value.trim()), true);
        } else if (!input.value.trim()) {
          uLoad('https://api.unsplash.com/photos?per_page=30', true);
        }
      }, 300);
    });

    // Scrolling to a screenful of the end brings the next page in.
    var body = uModal.querySelector('.u-body');
    body.addEventListener('scroll', function () {
      if (uZoomed || !uNext || uLoading) { return; }
      if (body.scrollTop + body.clientHeight >= body.scrollHeight - 1000) {
        uLoad(uNext, false);
      }
    });

    // Escape steps out of a zoom before it closes the picker, as Ghost's does.
    uModal.addEventListener('cancel', function (event) {
      if (uZoomed) {
        event.preventDefault();
        uUnzoom();
      }
    });
    uModal.addEventListener('close', function () { uUnzoom(); });
    uModal.querySelector('.u-close').addEventListener('click', function () { uModal.close(); });
    return uModal;
  }

  function uOpen() {
    var modal = unsplashModal();
    if (!modal.open) {
      modal.showModal();
      modal.querySelector('input').focus();
    }
    if (!config.unsplash) {
      // The option is always offered; without a key the picker says what is
      // missing instead of pretending to search.
      uSay('Turn Unsplash on under Settings → Apps to search and pick free photos.');
      return;
    }
    uLoad('https://api.unsplash.com/photos?per_page=30', true);
  }

  function uGrid() { return unsplashModal().querySelector('.u-grid'); }

  function uBusy(on) {
    uLoading = on;
    var spinner = unsplashModal().querySelector('.u-loading');
    if (spinner) { spinner.hidden = !on; }
  }

  // One line of prose where the grid goes, for waiting, empties and errors.
  function uSay(text) {
    uGrid().textContent = '';
    var row = document.createElement('div');
    row.className = 'u-empty';
    row.textContent = text;
    uGrid().appendChild(row);
  }

  function uLoad(url, fresh) {
    var mine = ++uSeq;
    if (fresh) {
      uNext = null;
      uZoomed && uUnzoom();
      uBusy(true);
      uSay(' ');
    } else {
      uBusy(true);
    }
    if (uController) { uController.abort(); }
    uController = new AbortController();

    fetch(url, {headers: config.unsplash || {}, signal: uController.signal})
      .then(function (response) {
        if (response.status === 403 && response.headers.get('x-ratelimit-remaining') === '0') {
          throw new Error('Unsplash API rate limit reached, please try again later.');
        }
        if (!response.ok) {
          return response.json().catch(function () { return null; }).then(function (json) {
            var message = json && json.errors && json.errors[0];
            throw new Error(message || ('Error ' + response.status + ': trouble reaching the Unsplash API'));
          });
        }
        // The next page, when there is one, comes straight off the Link
        // header — the API's own pagination, not a page counter.
        var next = null;
        var link = response.headers.get('link') || '';
        link.split(',').forEach(function (part) {
          var match = /<(.*)>; rel="next"/.exec(part.trim());
          if (match) { next = match[1]; }
        });
        uNext = next;
        return response.json();
      })
      .then(function (json) {
        if (mine !== uSeq) { return; }
        var photos = Array.isArray(json) ? json : (json && json.results) || [];
        if (fresh) {
          uGrid().textContent = '';
          if (!photos.length) { uSay('No photos matched. Try another search.'); }
        }
        photos.forEach(function (photo) { uGrid().appendChild(uTile(photo)); });
        if (!fresh && !photos.length) { uNext = null; }
      })
      .catch(function (err) {
        if (err && err.name === 'AbortError') { return; }
        if (mine !== uSeq) { return; }
        if (fresh) { uSay(err.message || 'Unsplash could not be reached.'); }
      })
      .then(function () {
        if (mine === uSeq) { uBusy(false); }
      });
  }

  // One masonry tile: the photograph at its own shape, over the colour the
  // API gives it, with Ghost's hover laid on top.
  function uTile(photo, zoom) {
    var tile = document.createElement('div');
    tile.className = 'u-tile' + (zoom ? ' zoomed' : '');
    tile.style.background = photo.color || 'transparent';

    var img = document.createElement('img');
    img.loading = 'lazy';
    img.alt = photo.alt_description || '';
    img.src = (photo.urls && photo.urls.regular) || '';
    tile.appendChild(img);

    var hover = document.createElement('div');
    hover.className = 'u-hover';
    var likes = document.createElement('a');
    likes.className = 'u-chip';
    likes.target = '_blank';
    likes.rel = 'noopener noreferrer';
    likes.href = (photo.links && uReferral(photo.links.html)) || '#';
    likes.innerHTML = U_HEART + '<span>' + (photo.likes || 0) + '</span>';
    var down = document.createElement('a');
    down.className = 'u-chip';
    down.target = '_blank';
    down.rel = 'noopener noreferrer';
    down.href = (photo.links && uReferral(photo.links.download)) + (photo.links && photo.links.download ? '&force=true' : '#');
    down.setAttribute('aria-label', 'Download');
    down.innerHTML = U_DOWNLOAD;
    var top = document.createElement('div');
    top.className = 'u-top';
    top.append(likes, down);

    var user = photo.user || {};
    var credit = document.createElement('span');
    credit.className = 'u-credit';
    if (user.profile_image && user.profile_image.medium) {
      var avatar = document.createElement('img');
      avatar.className = 'u-avatar';
      avatar.src = user.profile_image.medium;
      avatar.alt = '';
      credit.appendChild(avatar);
    }
    credit.appendChild(document.createTextNode(user.name || 'Unsplash'));

    var insert = document.createElement('button');
    insert.className = 'u-insert';
    insert.type = 'button';
    insert.textContent = 'Insert image';
    insert.addEventListener('click', function (event) {
      event.preventDefault();
      event.stopPropagation();
      uInsert(photo);
    });

    var bottom = document.createElement('div');
    bottom.className = 'u-bottom';
    bottom.append(credit, insert);
    hover.append(top, bottom);
    tile.appendChild(hover);

    if (!zoom) {
      tile.addEventListener('click', function () { uZoom(photo); });
    }
    return tile;
  }

  function uZoom(photo) {
    uZoomed = photo;
    var pane = unsplashModal().querySelector('.u-zoom');
    pane.textContent = '';
    pane.appendChild(uTile(photo, true));
    pane.hidden = false;
    uGrid().hidden = true;
    // Clicking the zoomed photograph is how you step back out of it.
    pane.addEventListener('click', uUnzoom);
  }

  function uUnzoom() {
    if (!uZoomed) { return; }
    uZoomed = null;
    var pane = unsplashModal().querySelector('.u-zoom');
    pane.removeEventListener('click', uUnzoom);
    pane.hidden = true;
    pane.textContent = '';
    uGrid().hidden = false;
  }

  // What Insert writes: the same wide URL and the same credit caption
  // Ghost's picker writes, referral links and all.
  function uInsert(photo) {
    var user = photo.user || {};
    var src = ((photo.urls && photo.urls.regular) || '').replace(/&w=1080/, '&w=2000');
    var credit = '<span>Photo by <a href="' + uReferral(user.links && user.links.html || 'https://unsplash.com/') + '">' +
      uEsc(user.name || 'Unsplash') + '</a> / <a href="' + uReferral('https://unsplash.com/') + '">Unsplash</a></span>';
    setFeature(src, credit);
    // Unsplash's guidelines ask each use to be reported; Ghost triggers this
    // endpoint on insert and so does this.
    if (photo.links && photo.links.download_location) {
      fetch(photo.links.download_location, {headers: config.unsplash || {}}).catch(function () {});
    }
    uModal.close();
  }

  wireFeatureImage();
})();
