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
  if (!Koenig || !React || !ReactDOM || !window.GhostKoenig) {
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

  function convertToHtml(editor, serialized) {
    var html = '';
    // Card subclasses serialize their nested editors into HTML in exportJSON.
    // exportDOM on the live node reads the original backing fields instead,
    // which can still contain the text from before those editors were edited.
    // Rehydrate the exact state being saved so every card exports those current
    // values, including captions, callouts, toggles, headers and CTA text.
    var renderState = editor.parseEditorState(serialized);
    renderState.read(function () {
      // $getRoot() is not exported by the UMD bundle, but every node can walk
      // up to the root it belongs to.
      var first = renderState._nodeMap.get('root');
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

  var integrations = window.GhostKoenig.create(config, banner);

  // The Form card (koenig-forms.js). Null when that script is absent or the
  // bundle's shape is not one it recognises, in which case the editor runs
  // with the built-in cards only.
  var formCard = null;
  try {
    formCard = window.GhostKoenigForms ? window.GhostKoenigForms.create(Koenig, React, config) : null;
  } catch (error) {
    console.error(error);
    formCard = null;
  }

  // Snapshot the live editor independently of the hidden inputs. A settings
  // save may have cleared those inputs, and previewing must never trigger save.
  form.ghostPreviewData = function () {
    var fields = new URLSearchParams();
    new FormData(form).forEach(function (value, key) {
      if (typeof value === 'string') fields.append(key, value);
    });
    if (api) {
      var serialized = api.serialize();
      fields.set('lexical', serialized);
      fields.set('html', convertToHtml(api.editorInstance, serialized));
      fields.delete('markdown');
    }
    return fields;
  };

  function Editor() {
    return e(
      'div',
      {className: 'koenig-react-editor koenig-lexical'},
      e(
        Koenig.KoenigComposer,
        {
          initialEditorState: config.lexical || undefined,
          // undefined leaves Koenig's own default node list in place
          nodes: formCard ? formCard.nodes : undefined,
          fileUploader: integrations.fileUploader,
          cardConfig: integrations.useCardConfig(),
          onError: function (error) { console.error(error); },
          darkMode: document.documentElement.classList.contains('dark'),
          isTKEnabled: false
        },
        e(Koenig.KoenigEditor, {
          placeholderText: config.placeholder || 'Begin writing your post...',
          placeholderClassName: 'editor-body-placeholder',
          registerAPI: function (editorApi) {
            api = editorApi;
            if (formCard && editorApi) { formCard.register(editorApi.editorInstance); }
          },
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
  // Set once the server has refused a save because another session wrote this
  // post. Autosave stays off from then on: the publisher's words are still in
  // the editor, and silently retrying would either keep failing or — once they
  // reloaded elsewhere — overwrite the other version.
  var collided = false;
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

    // Move the collision stamp forward to the version this save produced.
    // Without this the second save of a session would be compared against the
    // copy first loaded and refused as somebody else's work.
    var stampInput = document.getElementById('field-updated-at');
    if (stampInput && json.updated_at) { stampInput.value = json.updated_at; }

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
          htmlInput.value = convertToHtml(api.editorInstance, lexicalInput.value);
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
        // 409 is the update-collision refusal: somebody else saved this post
        // while this editor was open. Flagged rather than thrown plainly so
        // autosave stops instead of retrying into the same wall every few
        // seconds, and so the publisher is told how to keep their work.
        if (response.status === 409) {
          var collision = new Error((json && json.message) ||
            'Saving failed! Someone else is editing this post.');
          collision.collision = true;
          throw collision;
        }
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
      if (err && err.collision) {
        collided = true;
        clearTimeout(timer);
        timer = null;
        banner(err.message + ' Copy your changes, then reload to see their version.', 'error');
        return;
      }
      banner((err && err.message) || 'The save failed.', 'error');
    });
  }

  // The body or the title changed. A draft's edits start the autosave clock;
  // anything already out keeps them until Save, and says so.
  function contentEdited() {
    revision++;
    dirty = true;
    clearTimeout(timer);
    if (collided) {
      // No clock: an explicit save is the only way out, and it will be
      // refused again until the editor is reloaded.
      setState('* Unsaved changes', 'unsaved');
      return;
    }
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
      unsplashBtn.addEventListener('click', function () {
        window.GhostUnsplash.open({headers: config.unsplash, onSelect: setFeature});
      });
    }
    paintFeature();
  }


  wireFeatureImage();
})();
