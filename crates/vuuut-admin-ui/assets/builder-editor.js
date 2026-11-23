/* Page Builder & Visual Editor (v2).
 *
 * The page is shown as the theme draws it: the canvas iframe loads a
 * preview with `?vb=canvas`, which leaves an empty `[data-vb-canvas]` root,
 * and the layout is drawn into it by the SERVER (`endpoints.render`) — the
 * same renderer the site uses, expanded the same way, so what is seen here
 * is what is published. This script owns the model (sections → columns →
 * widgets, with inner sections), the panels — generated from the server's
 * field spec (`BOOT.spec`) — selection, drag and drop, inline text editing,
 * undo, the clipboard, templates and saving.
 *
 * The server is the authority: every save and every canvas draw is rebuilt
 * by `normalise` first. Text from the layout reaches the DOM only through
 * the server's renderer or `textContent`.
 */
(function () {
  'use strict';

  var BOOT = JSON.parse(document.getElementById('vb-boot').textContent);
  var SPEC = BOOT.spec;
  var ICONS = {};
  BOOT.icons.forEach(function (p) { ICONS[p[0]] = p[1]; });
  var WSPEC = {};
  SPEC.widgets.forEach(function (w) { WSPEC[w.key] = w; });
  var IS_TEMPLATE = BOOT.target.kind === 'template';
  var T = BOOT.target;

  var S = {
    layout: BOOT.layout,
    active: BOOT.active,
    showTitle: !!BOOT.post.showTitle,
    name: IS_TEMPLATE ? T.name : '',
    options: IS_TEMPLATE ? T.options : {},
    sel: null,
    tab: 'add',
    subtab: 'c',
    device: 'desktop',
    dirty: !!BOOT.isNew,
    saving: false,
    undo: [],
    redo: [],
    templates: BOOT.templates || [],
    refs: BOOT.refs || [],
    search: ''
  };
  var doc = null, root = null, frame = null, DRAG = null, EDITING = null;

  // ---------------------------------------------------------------------
  // helpers
  // ---------------------------------------------------------------------
  function $(s, r) { return (r || document).querySelector(s); }
  function esc(s) {
    return String(s == null ? '' : s).replace(/[&<>"']/g, function (c) {
      return {'&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;'}[c];
    });
  }
  function uid() { return 'a' + Math.random().toString(36).slice(2, 10) + Date.now().toString(36).slice(-3); }
  function clone(o) { return JSON.parse(JSON.stringify(o)); }
  function h(tag, attrs, kids) {
    var e = document.createElement(tag);
    if (attrs) Object.keys(attrs).forEach(function (k) {
      var v = attrs[k];
      if (v == null || v === false) return;
      if (k === 'class') e.className = v;
      else if (k === 'text') e.textContent = v;
      else if (k === 'html') e.innerHTML = v;
      else if (k.slice(0, 2) === 'on') e.addEventListener(k.slice(2), v);
      else e.setAttribute(k, v === true ? '' : v);
    });
    (kids || []).forEach(function (c) { if (c != null) e.appendChild(typeof c === 'string' ? document.createTextNode(c) : c); });
    return e;
  }
  function cssEsc(s) { return String(s).replace(/[^a-zA-Z0-9_-]/g, ''); }

  // UI icons (Material, 24-unit grid)
  var UI = {
    back: 'M20 11H7.83l5.59-5.59L12 4l-8 8 8 8 1.41-1.41L7.83 13H20v-2z',
    undo: 'M12.5 8c-2.65 0-5.05.99-6.9 2.6L2 7v9h9l-3.62-3.62c1.39-1.16 3.16-1.88 5.12-1.88 3.54 0 6.55 2.31 7.6 5.5l2.37-.78C21.08 11.03 17.15 8 12.5 8z',
    redo: 'M18.4 10.6C16.55 8.99 14.15 8 11.5 8c-4.65 0-8.58 3.03-9.96 7.22L3.9 16c1.05-3.19 4.05-5.5 7.6-5.5 1.95 0 3.73.72 5.12 1.88L13 16h9V7l-3.6 3.6z',
    desktop: 'M21 2H3c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h7v2H8v2h8v-2h-2v-2h7c1.1 0 2-.9 2-2V4c0-1.1-.9-2-2-2zm0 14H3V4h18v12z',
    tablet: 'M18.5 0h-14A2.5 2.5 0 0 0 2 2.5v19A2.5 2.5 0 0 0 4.5 24h14a2.5 2.5 0 0 0 2.5-2.5v-19A2.5 2.5 0 0 0 18.5 0zm-7 23c-.83 0-1.5-.67-1.5-1.5s.67-1.5 1.5-1.5 1.5.67 1.5 1.5-.67 1.5-1.5 1.5zm7.5-4H4V3h15v16z',
    mobile: 'M15.5 1h-8A2.5 2.5 0 0 0 5 3.5v17A2.5 2.5 0 0 0 7.5 23h8a2.5 2.5 0 0 0 2.5-2.5v-17A2.5 2.5 0 0 0 15.5 1zm-4 21c-.83 0-1.5-.67-1.5-1.5s.67-1.5 1.5-1.5 1.5.67 1.5 1.5-.67 1.5-1.5 1.5zm4.5-4H7V4h9v14z',
    add: 'M19 13h-6v6h-2v-6H5v-2h6V5h2v6h6v2z',
    edit: 'M3 17.25V21h3.75L17.81 9.94l-3.75-3.75L3 17.25zM20.71 7.04a1 1 0 0 0 0-1.41l-2.34-2.34a1 1 0 0 0-1.41 0l-1.83 1.83 3.75 3.75 1.83-1.83z',
    layers: 'M11.99 18.54l-7.37-5.73L3 14.07l9 7 9-7-1.63-1.27-7.38 5.74zM12 16l7.36-5.73L21 9l-9-7-9 7 1.63 1.27L12 16z',
    page: 'M14 2H6c-1.1 0-2 .9-2 2v16c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V8l-6-6zm2 16H8v-2h8v2zm0-4H8v-2h8v2zm-3-5V3.5L18.5 9H13z',
    eye: 'M12 4.5C7 4.5 2.73 7.61 1 12c1.73 4.39 6 7.5 11 7.5s9.27-3.11 11-7.5c-1.73-4.39-6-7.5-11-7.5zM12 17a5 5 0 1 1 0-10 5 5 0 0 1 0 10zm0-8a3 3 0 1 0 0 6 3 3 0 0 0 0-6z',
    up: 'M7.41 15.41 12 10.83l4.59 4.58L18 14l-6-6-6 6z',
    down: 'M7.41 8.59 12 13.17l4.59-4.58L18 10l-6 6-6-6z',
    copy: 'M16 1H4c-1.1 0-2 .9-2 2v14h2V3h12V1zm3 4H8c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h11c1.1 0 2-.9 2-2V7c0-1.1-.9-2-2-2zm0 16H8V7h11v14z',
    del: 'M6 19c0 1.1.9 2 2 2h8c1.1 0 2-.9 2-2V7H6v12zM19 4h-3.5l-1-1h-5l-1 1H5v2h14V4z',
    close: 'M19 6.41 17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z',
    check: 'M9 16.17 4.83 12l-1.42 1.41L9 19 21 7l-1.41-1.41z',
    history: 'M13 3a9 9 0 0 0-9 9H1l3.89 3.89.07.14L9 12H6c0-3.87 3.13-7 7-7s7 3.13 7 7-3.13 7-7 7c-1.93 0-3.68-.79-4.94-2.06l-1.42 1.42A8.95 8.95 0 0 0 13 21a9 9 0 0 0 0-18zm-1 5v5l4.28 2.54.72-1.21-3.5-2.08V8H12z',
    panel: 'M3 5v14h18V5H3zm6 12H5V7h4v10zm10 0h-8V7h8v10z',
    al: 'M15 15H3v2h12v-2zm0-8H3v2h12V7zM3 13h18v-2H3v2zm0 8h18v-2H3v2zM3 3v2h18V3H3z',
    ac: 'M7 15v2h10v-2H7zm-4 6h18v-2H3v2zm0-8h18v-2H3v2zm4-6v2h10V7H7zM3 3v2h18V3H3z',
    ar: 'M3 21h18v-2H3v2zm6-4h12v-2H9v2zm-6-4h18v-2H3v2zm6-4h12V7H9v2zM3 3v2h18V3H3z',
    aj: 'M3 21h18v-2H3v2zm0-4h18v-2H3v2zm0-4h18v-2H3v2zm0-4h18V7H3v2zm0-6v2h18V3H3z',
    upload: 'M5 20h14v-2H5v2zm0-10h4v6h6v-6h4l-7-7-7 7z',
    image: 'M21 19V5c0-1.1-.9-2-2-2H5c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h14c1.1 0 2-.9 2-2zM8.5 13.5l2.5 3.01L14.5 12l4.5 6H5l3.5-4.5z',
    link: 'M3.9 12c0-1.71 1.39-3.1 3.1-3.1h4V7H7a5 5 0 0 0 0 10h4v-1.9H7c-1.71 0-3.1-1.39-3.1-3.1zM8 13h8v-2H8v2zm9-6h-4v1.9h4c1.71 0 3.1 1.39 3.1 3.1s-1.39 3.1-3.1 3.1h-4V17h4a5 5 0 0 0 0-10z',
    drag: 'M11 18c0 1.1-.9 2-2 2s-2-.9-2-2 .9-2 2-2 2 .9 2 2zm-2-8c-1.1 0-2 .9-2 2s.9 2 2 2 2-.9 2-2-.9-2-2-2zm0-6c-1.1 0-2 .9-2 2s.9 2 2 2 2-.9 2-2-.9-2-2-2zm6 4c1.1 0 2-.9 2-2s-.9-2-2-2-2 .9-2 2 .9 2 2 2zm0 2c-1.1 0-2 .9-2 2s.9 2 2 2 2-.9 2-2-.9-2-2-2zm0 6c-1.1 0-2 .9-2 2s.9 2 2 2 2-.9 2-2-.9-2-2-2z',
    section: 'M3 5h18v6H3zm0 8h18v6H3z',
    column: 'M3 3h8v18H3zm10 0h8v18h-8z',
    save: 'M17 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14c1.1 0 2-.9 2-2V7l-4-4zm-5 16c-1.66 0-3-1.34-3-3s1.34-3 3-3 3 1.34 3 3-1.34 3-3 3zm3-10H5V5h10v4z',
    keyboard: 'M20 5H4c-1.1 0-1.99.9-1.99 2L2 17c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V7c0-1.1-.9-2-2-2zm-9 3h2v2h-2V8zm0 3h2v2h-2v-2zM8 8h2v2H8V8zm0 3h2v2H8v-2zm-1 2H5v-2h2v2zm0-3H5V8h2v2zm9 7H8v-2h8v2zm0-4h-2v-2h2v2zm0-3h-2V8h2v2zm3 3h-2v-2h2v2zm0-3h-2V8h2v2z',
    // widget glyphs
    heading: 'M5 4v3h5.5v12h3V7H19V4z',
    text: 'M14 17H4v2h10v-2zm6-8H4v2h16V9zM4 15h16v-2H4v2zM4 5v2h16V5H4z',
    button: 'M19 7H5c-1.1 0-2 .9-2 2v6c0 1.1.9 2 2 2h14c1.1 0 2-.9 2-2V9c0-1.1-.9-2-2-2zm0 8H5V9h14v6zM7 11h10v2H7z',
    feature: 'M12 2l2.4 7.6L22 12l-7.6 2.4L12 22l-2.4-7.6L2 12l7.6-2.4z',
    list: 'M3 13h2v-2H3v2zm0 4h2v-2H3v2zm0-8h2V7H3v2zm4 4h14v-2H7v2zm0 4h14v-2H7v2zM7 7v2h14V7H7z',
    spacer: 'M8 19h3v3h2v-3h3l-4-4-4 4zm8-15h-3V1h-2v3H8l4 4 4-4zM4 10v2h16v-2H4z',
    divider: 'M3 11h18v2H3z',
    video: 'M10 16.5l6-4.5-6-4.5v9zM12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm0 18c-4.41 0-8-3.59-8-8s3.59-8 8-8 8 3.59 8 8-3.59 8-8 8z',
    gallery: 'M22 16V4c0-1.1-.9-2-2-2H8c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2zm-11-4l2.03 2.71L16 11l4 5H8l3-4zM2 6v14c0 1.1.9 2 2 2h14v-2H4V6H2z',
    testimonial: 'M6 17h3l2-4V7H5v6h3zm8 0h3l2-4V7h-6v6h3z',
    accordion: 'M3 4h18v4H3zm0 6h18v2H3zm0 4h18v2H3zm0 4h18v2H3z',
    stat: 'M5 9.2h3V19H5zM10.6 5h2.8v14h-2.8zm5.6 8H19v6h-2.8z',
    pricing: 'M11.8 10.9c-2.27-.59-3-1.2-3-2.15 0-1.09 1.01-1.85 2.7-1.85 1.78 0 2.44.85 2.5 2.1h2.21c-.07-1.72-1.12-3.3-3.21-3.81V3h-3v2.16c-1.94.42-3.5 1.68-3.5 3.61 0 2.31 1.91 3.46 4.7 4.13 2.5.6 3 1.48 3 2.41 0 .69-.49 1.79-2.7 1.79-2.06 0-2.87-.92-2.98-2.1h-2.2c.12 2.19 1.76 3.42 3.68 3.83V21h3v-2.15c1.95-.37 3.5-1.5 3.5-3.55 0-2.84-2.43-3.81-4.7-4.4z',
    social: 'M18 16.08c-.76 0-1.44.3-1.96.77L8.91 12.7c.05-.23.09-.46.09-.7s-.04-.47-.09-.7l7.05-4.11A2.99 2.99 0 0 0 21 5a3 3 0 1 0-5.91.7L8.04 9.81A2.99 2.99 0 0 0 3 12a3 3 0 0 0 5.04 2.19l7.12 4.16c-.05.21-.08.43-.08.65A2.92 2.92 0 1 0 18 16.08z',
    map: 'M20.5 3l-.16.03L15 5.1 9 3 3.36 4.9c-.21.07-.36.25-.36.48V20.5a.5.5 0 0 0 .5.5l.16-.03L9 18.9l6 2.1 5.64-1.9c.21-.07.36-.25.36-.48V3.5a.5.5 0 0 0-.5-.5zM15 19l-6-2.11V5l6 2.11V19z',
    html: 'M9.4 16.6 4.8 12l4.6-4.6L8 6l-6 6 6 6 1.4-1.4zm5.2 0 4.6-4.6-4.6-4.6L16 6l6 6-6 6-1.4-1.4z'
  };
  UI.search = 'M15.5 14h-.79l-.28-.27A6.47 6.47 0 0 0 16 9.5 6.5 6.5 0 1 0 9.5 16c1.61 0 3.09-.59 4.23-1.57l.27.28v.79l5 4.99L20.49 19l-4.99-5zm-6 0C7.01 14 5 11.99 5 9.5S7.01 5 9.5 5 14 7.01 14 9.5 11.99 14 9.5 14z';
  UI.palette = 'M12 3a9 9 0 0 0 0 18c.83 0 1.5-.67 1.5-1.5 0-.39-.15-.74-.39-1.01-.23-.26-.38-.61-.38-.99 0-.83.67-1.5 1.5-1.5H16a5 5 0 0 0 5-5c0-4.42-4.03-8-9-8zm-5.5 9a1.5 1.5 0 1 1 0-3 1.5 1.5 0 0 1 0 3zm3-4a1.5 1.5 0 1 1 0-3 1.5 1.5 0 0 1 0 3zm5 0a1.5 1.5 0 1 1 0-3 1.5 1.5 0 0 1 0 3zm3 4a1.5 1.5 0 1 1 0-3 1.5 1.5 0 0 1 0 3z';
  UI.star = 'M12 17.27 18.18 21l-1.64-7.03L22 9.24l-7.19-.61L12 2 9.19 8.63 2 9.24l5.46 4.73L5.82 21z';
  UI.layout = UI.section;
  // glyphs for widgets that have none of their own above
  var GLYPH = {
    'image-box': 'image', icon: 'feature', anchor: 'link', inner: 'column', audio: 'video', carousel: 'gallery', slides: 'gallery',
    playlist: 'video', lottie: 'feature', embed: 'html', hotspot: 'image', posts: 'list', testimonials: 'testimonial', tabs: 'accordion',
    progress: 'stat', rating: 'feature', alert: 'feature', 'price-list': 'pricing', flipbox: 'copy', cta: 'button', 'animated-heading': 'heading',
    countdown: 'history', toc: 'list', share: 'social', blockquote: 'testimonial', code: 'html', 'text-path': 'heading', 'progress-tracker': 'stat',
    'off-canvas': 'panel', 'pay-button': 'pricing', form: 'page', signup: 'button', login: 'button', search: 'eye', 'nav-menu': 'list',
    'site-logo': 'image', breadcrumbs: 'link', 'author-box': 'testimonial', 'post-nav': 'link', 'post-info': 'text', 'post-content': 'page',
    sitemap: 'list', products: 'pricing', 'directory-listings': 'list', template: 'copy', global: 'save'
  };
  function ui(name, cls) {
    var d = UI[name] || UI[GLYPH[name]] || UI.add;
    return '<svg class="i' + (cls ? ' ' + cls : '') + '" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="' + d + '"/></svg>';
  }
  function iconSvg(name) {
    var d = ICONS[name];
    return d ? '<svg viewBox="0 0 24 24" width="1em" height="1em" fill="currentColor" aria-hidden="true"><path d="' + d + '"/></svg>' : null;
  }
  function label(kind) { return (WSPEC[kind] || {}).label || kind; }

  // ---------------------------------------------------------------------
  // the model
  // ---------------------------------------------------------------------
  function sections() { return S.layout.sections; }

  /* Every node, with where it lives. Inner sections are widgets holding
     columns, so a column's parent may be a section or an inner widget. */
  function locate(id) {
    var found = null;
    function inCols(cols, sec, si, owner) {
      for (var ci = 0; ci < cols.length && !found; ci++) {
        var col = cols[ci];
        if (col.id === id) { found = {kind: 'column', node: col, list: cols, index: ci, sec: sec, si: si, owner: owner, col: col}; return; }
        for (var wi = 0; wi < col.items.length && !found; wi++) {
          var w = col.items[wi];
          if (w.id === id) { found = {kind: 'widget', node: w, list: col.items, index: wi, sec: sec, si: si, col: col, owner: owner, w: w}; return; }
          if (w.t === 'inner' && w.cols) inCols(w.cols, sec, si, w);
        }
      }
    }
    var ss = sections();
    for (var si = 0; si < ss.length && !found; si++) {
      if (ss[si].id === id) return {kind: 'section', node: ss[si], list: ss, index: si, sec: ss[si], si: si};
      inCols(ss[si].cols, ss[si], si, ss[si]);
    }
    return found;
  }
  function reid(node) {
    node.id = uid();
    (node.cols || []).forEach(reid);
    (node.items || []).forEach(reid);
    return node;
  }
  function newColumn(w) { return {id: uid(), s: w && w !== 1 ? {w: w} : {}, items: []}; }
  function newSection(widths) { return {id: uid(), s: {}, cols: (widths || [1]).map(newColumn)}; }
  function newWidget(type) {
    var w = {id: uid(), t: type, s: clone((WSPEC[type] || {}).defaults || {})};
    if (type === 'inner') w.cols = [newColumn(), newColumn()];
    return w;
  }

  var lastKey = null, lastTime = 0;
  function record(key) {
    var now = Date.now();
    if (key && key === lastKey && now - lastTime < 1200) { lastTime = now; dirty(true); return; }
    S.undo.push(JSON.stringify({layout: S.layout, sel: S.sel}));
    if (S.undo.length > 150) S.undo.shift();
    S.redo = [];
    lastKey = key; lastTime = now;
    dirty(true);
  }
  function restore(from, to) {
    if (!from.length) return;
    to.push(JSON.stringify({layout: S.layout, sel: S.sel}));
    var snap = JSON.parse(from.pop());
    S.layout = snap.layout;
    S.sel = snap.sel && locate(snap.sel) ? snap.sel : null;
    lastKey = null;
    dirty(true);
    renderAll();
  }
  function undo() { restore(S.undo, S.redo); }
  function redo() { restore(S.redo, S.undo); }

  function select(id, tab) {
    S.sel = id;
    if (tab !== false && id) S.tab = 'edit';
    markSelection();
    renderSide();
  }

  /* Where a click-to-add goes: after the selected widget, into the selected
     column, or the end of the last section. */
  function target() {
    var sel = S.sel && locate(S.sel);
    if (sel && sel.kind === 'widget') return {list: sel.list, index: sel.index + 1, inInner: !!(sel.owner && sel.owner.t === 'inner')};
    if (sel && sel.kind === 'column') return {list: sel.col.items, index: sel.col.items.length, inInner: !!(sel.owner && sel.owner.t === 'inner')};
    if (sel && sel.kind === 'section') { var c = sel.sec.cols[0]; return {list: c.items, index: c.items.length}; }
    if (!sections().length) sections().push(newSection());
    var last = sections()[sections().length - 1].cols[0];
    return {list: last.items, index: last.items.length};
  }

  function insertWidget(type, colId, index) {
    var w = newWidget(type);
    var dest;
    if (colId) {
      var loc = locate(colId);
      if (!loc || loc.kind !== 'column') return;
      if (type === 'inner' && loc.owner && loc.owner.t === 'inner') { toast('An inner section cannot hold another inner section.', true); return; }
      dest = {list: loc.col.items, index: index == null ? loc.col.items.length : index};
    } else {
      dest = target();
      if (type === 'inner' && dest.inInner) { toast('An inner section cannot hold another inner section.', true); return; }
    }
    record();
    dest.list.splice(Math.min(dest.index, dest.list.length), 0, w);
    S.sel = w.id; S.tab = 'edit'; S.subtab = 'c';
    renderAll().then(function () { scrollIntoView(w.id); });
    return w;
  }

  function insertSections(list, afterId, imported) {
    record();
    if (imported) {
      if (imported.replaceBlank) S.layout.sections = [];
      if (!S.layout.elementor && imported.layout.elementor) {
        S.layout.elementor = imported.layout.elementor;
        S.layout.page = Object.assign({}, imported.layout.page, S.layout.page);
      }
    }
    var at = sections().length;
    var loc = (afterId || S.sel) && locate(afterId || S.sel);
    if (loc) at = loc.si + 1;
    list = list.map(function (s) { return reid(clone(s)); });
    Array.prototype.splice.apply(sections(), [at, 0].concat(list));
    S.sel = list[list.length - 1].id; S.tab = 'edit'; S.subtab = 'c';
    renderAll().then(function () { scrollIntoView(list[0].id); });
  }

  function moveWidget(id, colId, index) {
    var from = locate(id), to = locate(colId);
    if (!from || from.kind !== 'widget' || !to || to.kind !== 'column') return;
    if (from.w.t === 'inner' && to.owner && to.owner.t === 'inner') { toast('An inner section cannot hold another inner section.', true); return; }
    if (from.w.t === 'inner' && to.owner === from.w) return;
    if (from.list === to.col.items && (index === from.index || index === from.index + 1)) return;
    record();
    from.list.splice(from.index, 1);
    if (from.list === to.col.items && index > from.index) index--;
    to.col.items.splice(Math.min(index, to.col.items.length), 0, from.w);
    S.sel = id;
    renderAll();
  }

  function duplicate(id) {
    var loc = locate(id);
    if (!loc) return;
    if (loc.kind === 'column' && loc.list.length >= 6) { toast('A row holds at most six columns.', true); return; }
    record();
    var copy = reid(clone(loc.node));
    loc.list.splice(loc.index + 1, 0, copy);
    S.sel = copy.id;
    renderAll();
  }

  function remove(id) {
    var loc = locate(id);
    if (!loc) return;
    record();
    if (loc.kind === 'column' && loc.list.length === 1) {
      if (loc.owner && loc.owner.t === 'inner') { var o = locate(loc.owner.id); o.list.splice(o.index, 1); }
      else sections().splice(loc.si, 1);
    } else loc.list.splice(loc.index, 1);
    S.sel = null;
    renderAll();
  }

  function shift(id, dir) {
    var loc = locate(id);
    if (!loc) return;
    var j = loc.index + dir;
    if (j < 0 || j >= loc.list.length) {
      if (loc.kind === 'widget' && !(loc.owner && loc.owner.t === 'inner')) {
        var ns = sections()[loc.si + dir];
        if (!ns) return;
        record();
        loc.list.splice(loc.index, 1);
        var col = ns.cols[Math.min(sections()[loc.si] ? 0 : 0, ns.cols.length - 1)];
        if (dir > 0) col.items.unshift(loc.w); else col.items.push(loc.w);
        renderAll().then(function () { scrollIntoView(id); });
      }
      return;
    }
    record();
    var t = loc.list[loc.index]; loc.list[loc.index] = loc.list[j]; loc.list[j] = t;
    renderAll().then(function () { scrollIntoView(id); });
  }

  function setStructure(id, widths) {
    var loc = locate(id);
    if (!loc) return;
    var owner = loc.kind === 'section' ? loc.sec : loc.node;
    if (!owner.cols) return;
    record();
    var cols = owner.cols;
    var next = widths.map(function (w, i) {
      var c = cols[i] || newColumn();
      c.s = c.s || {};
      if (w === 1) delete c.s.w; else c.s.w = w;
      return c;
    });
    for (var k = widths.length; k < cols.length; k++) next[next.length - 1].items = next[next.length - 1].items.concat(cols[k].items);
    owner.cols = next;
    renderAll();
  }

  function addColumn(id) {
    var loc = locate(id);
    if (!loc) return;
    var owner = loc.kind === 'section' ? loc.sec : loc.node;
    if (!owner.cols) return;
    if (owner.cols.length >= 6) { toast('A row holds at most six columns.', true); return; }
    record();
    var c = newColumn();
    owner.cols.push(c);
    S.sel = c.id;
    renderAll();
  }

  // ---------------------------------------------------------------------
  // clipboard (localStorage, so it works across pages)
  // ---------------------------------------------------------------------
  var CLIP_KEY = 'vuuut:builder-clipboard';
  function copyNode(id) {
    var loc = locate(id);
    if (!loc) return;
    try { localStorage.setItem(CLIP_KEY, JSON.stringify({kind: loc.kind, node: loc.node})); } catch (e) { /* quota */ }
    toast(({section: 'Section', column: 'Column', widget: label(loc.node.t)})[loc.kind] + ' copied.');
  }
  function clip() { try { return JSON.parse(localStorage.getItem(CLIP_KEY) || 'null'); } catch (e) { return null; } }
  function paste() {
    var c = clip();
    if (!c) { toast('Nothing copied yet.', true); return; }
    var node = reid(clone(c.node));
    if (c.kind === 'section') { insertSections([node]); return; }
    if (c.kind === 'column') {
      var loc = S.sel && locate(S.sel);
      var owner = loc ? (loc.kind === 'section' ? loc.sec : loc.kind === 'column' ? (loc.owner || loc.sec) : (loc.owner || loc.sec)) : sections()[sections().length - 1];
      if (!owner) return;
      if (owner.cols.length >= 6) { toast('A row holds at most six columns.', true); return; }
      record(); owner.cols.push(node); S.sel = node.id; renderAll(); return;
    }
    var dest = target();
    if (node.t === 'inner' && dest.inInner) { toast('An inner section cannot hold another inner section.', true); return; }
    record();
    dest.list.splice(dest.index, 0, node);
    S.sel = node.id;
    renderAll();
  }
  /* Paste style: every Style and Advanced setting of the copied element onto
     the selected one of the same kind. */
  function pasteStyle() {
    var c = clip(), loc = S.sel && locate(S.sel);
    if (!c || !loc) return;
    if (c.kind !== loc.kind || (c.kind === 'widget' && c.node.t !== loc.node.t)) { toast('Copy an element of the same type first.', true); return; }
    var fields = fieldsFor(loc);
    record();
    fields.forEach(function (f) {
      if (!f.key || f.group === 'c') return;
      [f.key, f.key + '_t', f.key + '_m'].forEach(function (k) {
        if (c.node.s && c.node.s[k] !== undefined) loc.node.s[k] = clone(c.node.s[k]); else delete loc.node.s[k];
      });
    });
    renderAll();
    toast('Style pasted.');
  }

  // ---------------------------------------------------------------------
  // the canvas
  // ---------------------------------------------------------------------
  var CANVAS_CSS = [
    'html{scroll-behavior:auto!important}',
    '.vb-editing{min-height:60vh}',
    '.vb-editing .vb-anim{opacity:1!important;transform:none!important;filter:none!important}',
    '.vb-editing a,.vb-editing summary{cursor:default}',
    '.vb-editing .vb-s:hover{box-shadow:inset 0 0 0 1px rgba(124,77,255,.55)}',
    '.vb-editing .vb-col:hover{box-shadow:inset 0 0 0 1px rgba(47,107,255,.45)}',
    '.vb-editing .vb-w:hover{box-shadow:0 0 0 1px rgba(255,26,117,.55)}',
    '.vb-editing .vb-w{cursor:pointer;min-height:8px}',
    '.vb-editing .vb-s.vbx-sel{box-shadow:inset 0 0 0 2px #7c4dff!important}',
    '.vb-editing .vb-col.vbx-sel{box-shadow:inset 0 0 0 2px #2f6bff!important}',
    '.vb-editing .vb-w.vbx-sel{box-shadow:0 0 0 2px #ff1a75!important}',
    '.vb-editing .vb-w.vbx-dragging{opacity:.35}',
    '.vb-editing .vb-empty{display:flex;align-items:center;justify-content:center;min-height:120px;border:2px dashed rgba(127,127,127,.35);border-radius:12px;color:rgba(127,127,127,.9);font:500 13px system-ui,sans-serif;background:rgba(127,127,127,.06)}',
    '.vb-editing .vb-hide-d,.vb-editing .vb-hide-t,.vb-editing .vb-hide-m{opacity:.35;outline:1px dashed rgba(127,127,127,.6)}',
    '@media (min-width:1025px){.vb-editing .vb-hide-d{display:block!important}}',
    '@media (min-width:641px) and (max-width:1024px){.vb-editing .vb-hide-t{display:block!important}}',
    '@media (max-width:640px){.vb-editing .vb-hide-m{display:block!important}}',
    '.vb-editing .vb-col-link{display:none}',
    '.vb-editing [style*="position:fixed"],.vb-editing .vb-pt-top,.vb-editing .vb-pt-bottom{position:relative!important}',
    '.vbx-ph{display:flex;align-items:center;justify-content:center;gap:8px;min-height:90px;border:2px dashed rgba(127,127,127,.35);border-radius:12px;color:rgba(127,127,127,.95);font:500 13px system-ui,sans-serif;cursor:pointer;transition:border-color .15s,background .15s}',
    '.vbx-ph:hover,.vbx-ph.vbx-over{border-color:#ff1a75;color:#ff1a75;background:rgba(255,26,117,.05)}',
    '.vbx-drop{height:4px;border-radius:4px;background:#ff1a75;box-shadow:0 0 0 3px rgba(255,26,117,.2);margin:-2px 0;pointer-events:none}',
    '.vbx-add{display:flex;justify-content:center;padding:22px 0 40px}',
    '.vbx-add button{display:inline-flex;align-items:center;gap:8px;height:44px;padding:0 20px;border-radius:22px;border:2px dashed rgba(127,127,127,.45);background:transparent;color:inherit;opacity:.8;font:600 14px system-ui,sans-serif;cursor:pointer}',
    '.vbx-add button:hover,.vbx-add.vbx-over button{border-color:#ff1a75;color:#ff1a75;opacity:1}',
    '.vbx-bar{position:absolute;z-index:2147483000;display:flex;align-items:center;gap:1px;height:26px;padding:0 2px;border-radius:7px 7px 7px 0;background:#ff1a75;color:#fff;font:600 11.5px/1 system-ui,sans-serif;box-shadow:0 4px 12px -4px rgba(0,0,0,.35);user-select:none}',
    '.vbx-bar.sec{background:#7c4dff}.vbx-bar.col{background:#2f6bff}',
    '.vbx-bar b{padding:0 8px 0 6px;white-space:nowrap;letter-spacing:.02em}',
    '.vbx-bar button{display:grid;place-items:center;width:24px;height:24px;border:0;border-radius:5px;background:none;color:#fff;cursor:pointer;padding:0}',
    '.vbx-bar button:hover{background:rgba(255,255,255,.22)}',
    '.vbx-bar svg{width:15px;height:15px}',
    '.vbx-bar [data-act=drag]{cursor:grab}',
    '[contenteditable=true]{outline:2px solid #2f6bff!important;outline-offset:3px;cursor:text!important}',
    '.vbx-rich{position:absolute;z-index:2147483001;display:flex;gap:1px;padding:3px;border-radius:9px;background:#201a1b;box-shadow:0 8px 24px -8px rgba(0,0,0,.5)}',
    '.vbx-rich button{min-width:28px;height:28px;border:0;border-radius:6px;background:none;color:#fff;font:700 13px system-ui,sans-serif;cursor:pointer;padding:0 6px}',
    '.vbx-rich button:hover{background:rgba(255,255,255,.18)}',
    '.vbx-first{max-width:560px;margin:60px auto;padding:40px 24px;text-align:center;border:2px dashed rgba(127,127,127,.4);border-radius:24px;font:15px/1.5 system-ui,sans-serif}',
    '.vbx-first b{display:block;font-size:20px;margin-bottom:6px}',
    '.vbx-busy{opacity:.55;transition:opacity .15s}'
  ].join('\n');

  function initCanvas() {
    frame = $('#vbe-canvas');
    var wrap = $('#vbe-frame');
    var loading = h('div', {class: 'vbe-loading', text: 'Opening the page in your theme…'});
    wrap.appendChild(loading);
    var triedFallback = false;
    function fallback() {
      triedFallback = true;
      frame.srcdoc = '<!doctype html><html><head><meta charset="utf-8"><link rel="stylesheet" href="' + esc(BOOT.cssUrl) + '"><script src="' + esc(BOOT.jsUrl) + '" defer><\/script><style>body{margin:0;font:18px/1.6 system-ui,-apple-system,sans-serif;color:#15171a;background:#fff}</style></head><body><div class="vb-page vb-editing" data-vb-canvas style="' + esc(BOOT.vars) + '"></div></body></html>';
    }
    frame.addEventListener('load', function () {
      var d;
      try { d = frame.contentDocument; } catch (e) { d = null; }
      var r = d && d.querySelector('[data-vb-canvas]');
      if (!r) {
        if (!triedFallback) { fallback(); return; }
        loading.textContent = 'The canvas could not be opened.';
        return;
      }
      loading.remove();
      setupCanvas(d, r);
    });
    if (BOOT.canvasUrl) frame.src = BOOT.canvasUrl; else fallback();
  }

  function setupCanvas(d, r) {
    doc = d; root = r;
    // popups and headers of the Theme Builder are the site's, not this page's
    var style = d.createElement('style');
    style.textContent = CANVAS_CSS;
    d.head.appendChild(style);
    d.addEventListener('submit', function (e) { e.preventDefault(); }, true);
    d.addEventListener('click', onCanvasClick, true);
    d.addEventListener('dblclick', onCanvasDbl, true);
    d.addEventListener('contextmenu', onContext, true);
    d.addEventListener('dragstart', onDragStart, true);
    d.addEventListener('dragover', onDragOver, true);
    d.addEventListener('dragleave', function (e) { if (e.target === d.documentElement) clearDrop(); });
    d.addEventListener('drop', onDrop, true);
    d.addEventListener('dragend', endDrag, true);
    d.addEventListener('keydown', onKey);
    d.defaultView.addEventListener('scroll', positionBar, {passive: true});
    d.defaultView.addEventListener('resize', positionBar);
    $('#vbe-app').removeAttribute('aria-busy');
    renderCanvas().then(function () {
      var top = root.getBoundingClientRect().top + d.defaultView.scrollY - 40;
      if (top > 200) d.defaultView.scrollTo(0, top);
    });
  }

  var drawSeq = 0, drawTimer = null, drawing = null;
  function renderCanvasSoon() {
    clearTimeout(drawTimer);
    drawTimer = setTimeout(renderCanvas, 140);
  }
  function renderCanvas() {
    if (!root) return Promise.resolve();
    if (EDITING) return Promise.resolve();
    clearTimeout(drawTimer);
    var seq = ++drawSeq;
    root.classList.add('vbx-busy');
    drawing = fetch(BOOT.endpoints.render, {method: 'POST', credentials: 'same-origin', headers: {'Content-Type': 'application/json', Accept: 'application/json'}, body: JSON.stringify({layout: S.layout})})
      .then(function (r) { return r.json().then(function (j) { if (!r.ok) throw new Error(j.error || 'The canvas could not be drawn (' + r.status + ').'); return j; }); })
      .then(function (j) {
        if (seq !== drawSeq || EDITING) return;
        // the server's HTML: the layout drawn as the site draws it. Its own
        // .vb-page wrapper is unwrapped into the canvas root.
        var tmp = doc.createElement('div');
        tmp.innerHTML = j.html;
        var page = tmp.querySelector('.vb-page');
        var out = '';
        Array.prototype.forEach.call(tmp.childNodes, function (n) { if (n !== page) out += n.outerHTML || ''; });
        if (page) {
          root.setAttribute('style', page.getAttribute('style') || '');
          root.className = page.className + ' vb-editing';
          out += page.innerHTML;
        }
        if (!sections().length) out += '<div class="vbx-first"><b>Start with a section</b>Pick a layout or a ready-made template from <i>Add</i> on the left, or drag a widget here.</div>';
        out += '<div class="vbx-add" data-vbx-add><button type="button">' + ui('add') + 'Add section</button></div>';
        root.innerHTML = out;
        markSelection();
        var rt = doc.defaultView.vuuutBuilder;
        if (rt) rt.init(root);
      })
      .catch(function (e) { toast(e.message, true); })
      .then(function () { if (seq === drawSeq) root.classList.remove('vbx-busy'); });
    return drawing;
  }

  function el(id) { return id && root ? root.querySelector('[data-vb-id="' + cssEsc(id) + '"]') : null; }

  function markSelection() {
    if (!root) return;
    Array.prototype.forEach.call(root.querySelectorAll('.vbx-sel'), function (e) { e.classList.remove('vbx-sel'); });
    var e = el(S.sel);
    if (e) e.classList.add('vbx-sel');
    positionBar();
  }

  function positionBar() {
    if (!doc) return;
    var bar = doc.querySelector('.vbx-bar');
    var loc = S.sel && locate(S.sel);
    var node = el(S.sel);
    if (!loc || !node || EDITING) { if (bar) bar.remove(); return; }
    if (!bar || bar.dataset.for !== S.sel) {
      if (bar) bar.remove();
      bar = doc.createElement('div');
      bar.dataset.for = S.sel;
      bar.className = 'vbx-bar' + (loc.kind === 'section' ? ' sec' : loc.kind === 'column' ? ' col' : '');
      var name = loc.kind === 'section' ? 'Section' : loc.kind === 'column' ? 'Column' : label(loc.w.t);
      var acts = [['up', 'Move up'], ['down', 'Move down'], ['copy', 'Duplicate'], ['del', 'Delete']];
      if (loc.kind === 'section' || (loc.kind === 'widget' && loc.w.t === 'inner')) acts.splice(2, 0, ['add', 'Add a column']);
      if (loc.kind === 'widget') acts.unshift(['drag', 'Drag to move']);
      bar.innerHTML = '<b>' + esc(name) + '</b>' + acts.map(function (a) { return '<button type="button" data-act="' + a[0] + '" title="' + a[1] + '"' + (a[0] === 'drag' ? ' draggable="true"' : '') + '>' + ui(a[0]) + '</button>'; }).join('');
      bar.addEventListener('click', function (e) {
        var b = e.target.closest('button');
        e.preventDefault(); e.stopPropagation();
        if (!b) return;
        var act = b.dataset.act, id = bar.dataset.for;
        if (act === 'up') shift(id, -1);
        else if (act === 'down') shift(id, 1);
        else if (act === 'copy') duplicate(id);
        else if (act === 'del') remove(id);
        else if (act === 'add') addColumn(id);
      }, true);
      doc.body.appendChild(bar);
    }
    var r = node.getBoundingClientRect(), win = doc.defaultView;
    var top = loc.kind === 'widget' && r.top >= 30 ? r.top + win.scrollY - 26 : r.top + win.scrollY;
    bar.style.top = Math.max(0, top) + 'px';
    var left = loc.kind === 'column' ? r.right + win.scrollX - bar.offsetWidth : r.left + win.scrollX;
    bar.style.left = Math.max(0, Math.min(left, win.scrollX + doc.documentElement.clientWidth - bar.offsetWidth - 2)) + 'px';
  }

  function idOf(node) {
    var n = node && node.closest && node.closest('[data-vb-id]');
    return n ? n.getAttribute('data-vb-id') : null;
  }

  function onCanvasClick(e) {
    if (EDITING && EDITING.el.contains(e.target)) return;
    if (e.target.closest && (e.target.closest('.vbx-rich') || e.target.closest('.vbx-bar'))) return;
    hideMenu();
    e.preventDefault();
    if (EDITING) finishEditing();
    var add = e.target.closest && e.target.closest('[data-vbx-add]');
    if (add) { S.tab = 'add'; S.sel = null; markSelection(); renderSide(); flash($('.vbe-structs')); return; }
    var ph = e.target.closest && e.target.closest('[data-vbx-col]');
    if (ph) { S.sel = ph.getAttribute('data-vbx-col'); S.tab = 'add'; markSelection(); renderSide(); return; }
    if (!root.contains(e.target)) return;
    var id = idOf(e.target);
    if (!id || !locate(id)) return;
    var loc = locate(id);
    if (loc.kind === 'widget' && S.sel === id && (loc.w.t === 'heading' || loc.w.t === 'text')) { startEditing(id); return; }
    select(id);
  }
  function onCanvasDbl(e) {
    var id = idOf(e.target);
    var loc = id && locate(id);
    if (loc && loc.kind === 'widget' && (loc.w.t === 'heading' || loc.w.t === 'text')) {
      e.preventDefault();
      S.sel = id;
      startEditing(id);
    }
  }

  // ---- inline text editing ----
  function startEditing(id) {
    var loc = locate(id);
    var wel = el(id);
    if (!loc || !wel) return;
    var t = loc.w.t === 'heading' ? wel.querySelector('.vb-h') : wel.querySelector('.vb-text');
    if (!t) return;
    if (loc.w.t === 'heading') { var a = t.querySelector('a'); if (a) t = a; }
    // dynamic tags were filled in for the preview; edit the source instead
    if (loc.w.t === 'heading') t.textContent = loc.w.s.text || '';
    else t.innerHTML = loc.w.s.html || '';
    EDITING = {id: id, el: t, kind: loc.w.t, before: JSON.stringify(S.layout)};
    wel.removeAttribute('draggable');
    t.setAttribute('contenteditable', 'true');
    t.focus();
    var range = doc.createRange();
    range.selectNodeContents(t);
    range.collapse(false);
    var sel = doc.defaultView.getSelection();
    sel.removeAllRanges(); sel.addRange(range);
    t.addEventListener('input', onInlineInput);
    t.addEventListener('blur', function onBlur() {
      t.removeEventListener('blur', onBlur);
      setTimeout(function () { if (EDITING && EDITING.el === t && doc.activeElement !== t) finishEditing(); }, 150);
    });
    t.addEventListener('keydown', function (e) {
      if (EDITING && EDITING.kind === 'heading' && e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); finishEditing(); }
      if (e.key === 'Escape') { e.preventDefault(); finishEditing(); }
    });
    positionBar();
    if (loc.w.t === 'text') showRichBar(t);
    S.tab = 'edit';
    renderSide();
  }
  function onInlineInput() {
    if (!EDITING) return;
    var loc = locate(EDITING.id);
    if (!loc) return;
    if (EDITING.kind === 'heading') loc.w.s.text = EDITING.el.innerText.replace(/\n+$/, '');
    else loc.w.s.html = EDITING.el.innerHTML;
    dirty(true);
    var rb = doc.querySelector('.vbx-rich');
    if (rb) placeRich(rb, EDITING.el);
  }
  function finishEditing() {
    if (!EDITING) return;
    var ed = EDITING;
    EDITING = null;
    ed.el.removeAttribute('contenteditable');
    var rb = doc.querySelector('.vbx-rich');
    if (rb) rb.remove();
    if (JSON.stringify(S.layout) !== ed.before) {
      S.undo.push(JSON.stringify({layout: JSON.parse(ed.before), sel: ed.id}));
      S.redo = [];
      lastKey = null;
    }
    renderAll();
  }
  function richCommand(d, c, after) {
    if (c === 'link') {
      var url = window.prompt('Link address (https://…, /page/ or #anchor)', 'https://');
      if (url) d.execCommand('createLink', false, url);
    } else if (c === 'h3' || c === 'p' || c === 'h4' || c === 'blockquote') d.execCommand('formatBlock', false, c);
    else d.execCommand(c, false, null);
    after();
  }
  var RICH = [['bold', 'B', 'Bold'], ['italic', 'I', 'Italic'], ['underline', 'U', 'Underline'], ['strikeThrough', 'S', 'Strike'], ['link', '🔗', 'Link'], ['insertUnorderedList', '•', 'Bulleted list'], ['insertOrderedList', '1.', 'Numbered list'], ['h3', 'H3', 'Subheading'], ['blockquote', '“', 'Quote'], ['p', '¶', 'Paragraph'], ['removeFormat', '⨯', 'Clear formatting']];
  function showRichBar(t) {
    var bar = doc.createElement('div');
    bar.className = 'vbx-rich';
    RICH.forEach(function (c) {
      var b = doc.createElement('button');
      b.type = 'button'; b.textContent = c[1]; b.title = c[2];
      b.addEventListener('mousedown', function (e) { e.preventDefault(); richCommand(doc, c[0], onInlineInput); });
      bar.appendChild(b);
    });
    doc.body.appendChild(bar);
    placeRich(bar, t);
  }
  function placeRich(bar, t) {
    var r = t.getBoundingClientRect(), win = doc.defaultView;
    bar.style.top = Math.max(0, r.top + win.scrollY - 42) + 'px';
    bar.style.left = Math.max(4, r.left + win.scrollX) + 'px';
  }

  // ---- drag and drop ----
  function onDragStart(e) {
    var handle = e.target.closest && e.target.closest('.vbx-bar [data-act=drag]');
    var wel = handle ? el(handle.closest('.vbx-bar').dataset.for) : (e.target.closest && e.target.closest('.vb-w'));
    if (!wel || EDITING) { e.preventDefault(); return; }
    DRAG = {kind: 'move', id: wel.getAttribute('data-vb-id')};
    wel.classList.add('vbx-dragging');
    e.dataTransfer.effectAllowed = 'move';
    try { e.dataTransfer.setData('text/plain', 'vb:' + DRAG.id); } catch (x) { /* */ }
    e.stopPropagation();
  }
  function clearDrop() {
    if (!doc) return;
    Array.prototype.forEach.call(doc.querySelectorAll('.vbx-drop'), function (n) { n.remove(); });
    Array.prototype.forEach.call(doc.querySelectorAll('.vbx-over'), function (n) { n.classList.remove('vbx-over'); });
  }
  function dropTarget(e) {
    var t = e.target;
    if (!t.closest) t = t.parentElement;
    if (!t) return null;
    if (t.closest('[data-vbx-add], .vbx-first')) return {kind: 'new-section', el: doc.querySelector('[data-vbx-add]')};
    var col = t.closest('.vb-col');
    if (col && !root.contains(col)) col = null;
    if (!col) {
      var sec = t.closest('.vb-s');
      if (!sec || !root.contains(sec)) return root.contains(t) || t === root ? {kind: 'new-section', el: doc.querySelector('[data-vbx-add]')} : null;
      var cols = sec.querySelectorAll(':scope > .vb-in > .vb-row > .vb-col');
      var best = null, bd = Infinity;
      Array.prototype.forEach.call(cols, function (c) {
        var r = c.getBoundingClientRect(), d = Math.abs((r.left + r.right) / 2 - e.clientX);
        if (d < bd) { bd = d; best = c; }
      });
      col = best;
      if (!col) return null;
    }
    var ws = Array.prototype.filter.call(col.children, function (c) { return c.classList.contains('vb-w'); });
    var index = ws.length, before = null;
    for (var i = 0; i < ws.length; i++) {
      var r = ws[i].getBoundingClientRect();
      if (e.clientY < r.top + r.height / 2) { index = i; before = ws[i]; break; }
    }
    return {kind: 'column', colId: col.getAttribute('data-vb-id'), col: col, index: index, before: before};
  }
  function onDragOver(e) {
    if (!DRAG) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = DRAG.kind === 'move' ? 'move' : 'copy';
    var t = dropTarget(e);
    clearDrop();
    if (!t) return;
    if (t.kind === 'new-section') { if (t.el) t.el.classList.add('vbx-over'); return; }
    var ph = t.col.querySelector(':scope > .vbx-ph');
    if (ph) { ph.classList.add('vbx-over'); return; }
    var line = doc.createElement('div');
    line.className = 'vbx-drop';
    if (t.before) t.col.insertBefore(line, t.before); else t.col.appendChild(line);
  }
  function onDrop(e) {
    if (!DRAG) return;
    e.preventDefault();
    var t = dropTarget(e);
    clearDrop();
    var d = DRAG;
    endDrag();
    if (!t) return;
    if (t.kind === 'new-section') {
      record();
      var s = newSection();
      sections().push(s);
      lastKey = 'x';
      var w;
      if (d.kind === 'new') w = newWidget(d.type);
      else { var from = locate(d.id); if (!from) return; from.list.splice(from.index, 1); w = from.w; }
      s.cols[0].items.push(w);
      S.sel = w.id; S.tab = 'edit';
      renderAll().then(function () { scrollIntoView(w.id); });
      return;
    }
    if (d.kind === 'new') insertWidget(d.type, t.colId, t.index);
    else moveWidget(d.id, t.colId, t.index);
  }
  function endDrag() {
    DRAG = null;
    clearDrop();
    if (doc) Array.prototype.forEach.call(doc.querySelectorAll('.vbx-dragging'), function (n) { n.classList.remove('vbx-dragging'); });
  }

  function scrollIntoView(id) {
    var n = el(id);
    if (!n) return;
    var r = n.getBoundingClientRect(), win = doc.defaultView;
    if (r.top < 60 || r.bottom > win.innerHeight - 20) win.scrollTo({top: Math.max(0, r.top + win.scrollY - 120), behavior: 'smooth'});
  }

  // ---------------------------------------------------------------------
  // context menu
  // ---------------------------------------------------------------------
  var menuEl = null;
  function hideMenu() { if (menuEl) { menuEl.remove(); menuEl = null; } }
  function onContext(e) {
    var id = idOf(e.target);
    if (!id || !locate(id) || EDITING) return;
    e.preventDefault();
    select(id, false);
    var fr = frame.getBoundingClientRect();
    showMenu(id, fr.left + e.clientX, fr.top + e.clientY);
  }
  function showMenu(id, x, y) {
    hideMenu();
    var loc = locate(id);
    if (!loc) return;
    var c = clip();
    var items = [
      ['edit', 'Edit', function () { select(id); }],
      ['copy', 'Duplicate', function () { duplicate(id); }, '⌘D'],
      ['copy', 'Copy', function () { copyNode(id); }, '⌘C'],
      ['save', 'Paste', paste, '⌘V', !c],
      ['palette', 'Paste style', pasteStyle, '⇧⌘V', !c || c.kind !== loc.kind],
      null,
      ['up', 'Move up', function () { shift(id, -1); }],
      ['down', 'Move down', function () { shift(id, 1); }]
    ];
    if (loc.kind === 'section') items.push(['save', 'Save as template', function () { saveTemplate('section', {sections: [loc.sec]}); }, null, IS_TEMPLATE]);
    if (loc.kind === 'widget' && loc.w.t !== 'global') items.push(['save', 'Save as global widget', function () { saveGlobal(id); }, null, IS_TEMPLATE]);
    if (loc.kind === 'widget' && loc.w.t === 'global') items.push(['copy', 'Unlink global (make a copy)', function () { unlinkGlobal(id); }]);
    items.push(['layers', 'Navigator', function () { S.tab = 'layers'; renderSide(); }]);
    items.push(null);
    items.push(['del', 'Delete', function () { remove(id); }, 'Del']);
    menuEl = h('div', {class: 'vbe-menu', role: 'menu'});
    items.forEach(function (it) {
      if (!it) { menuEl.appendChild(h('hr')); return; }
      if (it[4]) return;
      menuEl.appendChild(h('button', {type: 'button', role: 'menuitem', html: ui(it[0]) + '<span>' + esc(it[1]) + '</span>' + (it[3] ? '<kbd>' + it[3] + '</kbd>' : ''), onclick: function () { hideMenu(); it[2](); }}));
    });
    document.body.appendChild(menuEl);
    var w = menuEl.offsetWidth, hgt = menuEl.offsetHeight;
    menuEl.style.left = Math.min(x, innerWidth - w - 8) + 'px';
    menuEl.style.top = Math.min(y, innerHeight - hgt - 8) + 'px';
    var first = menuEl.querySelector('button'); if (first) first.focus();
  }
  document.addEventListener('mousedown', function (e) { if (menuEl && !menuEl.contains(e.target)) hideMenu(); });

  function saveGlobal(id) {
    var loc = locate(id);
    if (!loc || loc.kind !== 'widget') return;
    var name = h('input', {class: 'vbe-in', maxlength: '120', value: label(loc.w.t)});
    dialog('Save as a global widget', 'Edit it once under Tools → Page Builder → Templates and every copy updates. This one becomes linked to it.', name, 'Save').then(function (ok) {
      if (!ok || !name.value.trim()) return;
      post(BOOT.endpoints.templates, {name: name.value.trim(), kind: 'widget', data: {v: 2, page: {}, sections: [{id: 's1', s: {}, cols: [{id: 'c1', s: {}, items: [loc.w]}]}]}}).then(function (j) {
        S.templates = j.templates || S.templates; S.refs = j.refs || S.refs;
        record();
        loc.list[loc.index] = {id: uid(), t: 'global', s: {ref: j.id}};
        S.sel = loc.list[loc.index].id;
        renderAll();
        toast('Saved as a global widget.');
      }).catch(function (e) { toast(e.message, true); });
    });
  }
  function unlinkGlobal(id) {
    var loc = locate(id);
    var t = S.templates.find(function (x) { return x.id === loc.w.s.ref; });
    var w = t && t.data && t.data.sections && t.data.sections[0] && t.data.sections[0].cols[0].items[0];
    if (!w) { toast('That global widget is not available.', true); return; }
    record();
    loc.list[loc.index] = reid(clone(w));
    S.sel = loc.list[loc.index].id;
    renderAll();
  }

  // ---------------------------------------------------------------------
  // top bar
  // ---------------------------------------------------------------------
  function renderTop() {
    var top = $('#vbe-top');
    top.innerHTML = '';
    var p = BOOT.post;
    top.appendChild(h('a', {class: 'vbe-brand', href: BOOT.toolUrl, title: 'Page Builder', text: 'v'}));
    top.appendChild(h('a', {class: 'vbe-ib', href: p.editUrl, title: IS_TEMPLATE ? 'Back to the list' : 'Back to the editor', html: ui('back')}));
    var sub = IS_TEMPLATE ? T.kindLabel + (T.theme ? ' · ' + (S.active ? 'On' : 'Off') : '') : (p.type === 'page' ? 'Page' : 'Post') + ' · ' + p.status.charAt(0).toUpperCase() + p.status.slice(1);
    top.appendChild(h('div', {class: 'vbe-title'}, [h('b', {text: IS_TEMPLATE ? S.name : (p.title || 'Untitled')}), h('span', {text: sub})]));
    top.appendChild(h('div', {class: 'vbe-group'}, [
      h('button', {class: 'vbe-ib', title: 'Show or hide the panel', html: ui('panel'), onclick: function () { $('#vbe-app').classList.toggle('vbe-collapsed'); setTimeout(positionBar, 50); }}),
      h('button', {class: 'vbe-ib', title: 'Finder (⌘K)', html: ui('search'), onclick: finder})
    ]));
    top.appendChild(h('div', {class: 'vbe-spacer'}));
    var dev = h('div', {class: 'vbe-group', role: 'group', 'aria-label': 'Device'});
    [['desktop', 'Desktop'], ['tablet', 'Tablet — 820px'], ['mobile', 'Mobile — 390px']].forEach(function (d) {
      dev.appendChild(h('button', {class: 'vbe-ib' + (S.device === d[0] ? ' on' : ''), title: d[1] + ' — settings marked with a device apply to it', 'aria-pressed': S.device === d[0] ? 'true' : 'false', html: ui(d[0]), onclick: function () { setDevice(d[0]); }}));
    });
    top.appendChild(dev);
    top.appendChild(h('div', {class: 'vbe-group'}, [
      h('button', {class: 'vbe-ib', id: 'vbe-undo', title: 'Undo (⌘Z)', html: ui('undo'), disabled: !S.undo.length, onclick: undo}),
      h('button', {class: 'vbe-ib', id: 'vbe-redo', title: 'Redo (⇧⌘Z)', html: ui('redo'), disabled: !S.redo.length, onclick: redo})
    ]));
    top.appendChild(h('div', {class: 'vbe-spacer'}));
    top.appendChild(h('span', {class: 'vbe-state', id: 'vbe-state', 'aria-live': 'polite', text: S.saving ? 'Saving…' : S.dirty ? 'Unsaved changes' : (BOOT.savedAt ? 'All saved' : '')}));
    if (!IS_TEMPLATE || T.theme) {
      top.appendChild(h('label', {class: 'vbe-live vbe-hide-narrow', title: IS_TEMPLATE ? 'When on, this part shows on the site wherever its conditions match.' : 'When on, visitors see this layout instead of the page’s own content.'}, [
        h('span', {class: 'vbe-switch'}, [h('input', {type: 'checkbox', checked: S.active, onchange: function (e) { S.active = e.target.checked; dirty(true); renderTop(); }}), h('span')]),
        document.createTextNode(S.active ? (IS_TEMPLATE ? 'On' : 'Layout on') : (IS_TEMPLATE ? 'Off' : 'Layout off'))
      ]));
    }
    if (!IS_TEMPLATE) top.appendChild(h('button', {class: 'vbe-btn text vbe-hide-narrow', title: 'Save, then open the page in a new tab', html: ui('eye') + ' Preview', onclick: preview}));
    top.appendChild(h('button', {class: 'vbe-btn primary', id: 'vbe-save', 'data-dirty': S.dirty ? '1' : '0', disabled: S.saving, html: ui('save') + ' Save', onclick: save}));
  }
  function setDevice(d) {
    S.device = d;
    $('#vbe-frame').setAttribute('data-device', d);
    renderTop();
    renderSide();
    setTimeout(positionBar, 380);
  }
  function dirty(v) {
    S.dirty = v;
    var b = $('#vbe-save'); if (b) b.setAttribute('data-dirty', v ? '1' : '0');
    var st = $('#vbe-state'); if (st) st.textContent = S.saving ? 'Saving…' : v ? 'Unsaved changes' : 'All saved';
    var u = $('#vbe-undo'); if (u) u.disabled = !S.undo.length;
    var r = $('#vbe-redo'); if (r) r.disabled = !S.redo.length;
  }

  // ---------------------------------------------------------------------
  // side panel
  // ---------------------------------------------------------------------
  function renderSide() {
    var side = $('#vbe-side');
    var old = side.querySelector('.vbe-pane');
    var scroll = old ? old.scrollTop : 0;
    var same = side.dataset.tab === S.tab && side.dataset.sel === (S.sel || '');
    var focusKey = document.activeElement && document.activeElement.dataset ? document.activeElement.dataset.fkey : null;
    side.innerHTML = '';
    var tabs = h('div', {class: 'vbe-tabs', role: 'tablist'});
    [['add', 'Add', 'add'], ['edit', 'Edit', 'edit'], ['layers', 'Layers', 'layers'], ['page', IS_TEMPLATE ? 'Settings' : 'Page', 'page']].forEach(function (t) {
      tabs.appendChild(h('button', {class: 'vbe-tab' + (S.tab === t[0] ? ' on' : ''), role: 'tab', 'aria-selected': S.tab === t[0] ? 'true' : 'false', html: ui(t[2]) + '<span>' + t[1] + '</span>', onclick: function () { if (EDITING) finishEditing(); S.tab = t[0]; renderSide(); }}));
    });
    side.appendChild(tabs);
    var pane = h('div', {class: 'vbe-pane'});
    side.appendChild(pane);
    if (S.tab === 'add') paneAdd(pane);
    else if (S.tab === 'edit') paneEdit(pane);
    else if (S.tab === 'layers') paneLayers(pane);
    else panePage(pane);
    side.dataset.tab = S.tab; side.dataset.sel = S.sel || '';
    if (same) pane.scrollTop = scroll;
    if (focusKey) { var f = pane.querySelector('[data-fkey="' + focusKey + '"]'); if (f) { f.focus(); if (f.setSelectionRange && f.value != null) try { f.setSelectionRange(f.value.length, f.value.length); } catch (e) { /* */ } } }
  }

  function paneAdd(pane) {
    var search = h('input', {class: 'vbe-search', type: 'search', placeholder: 'Search ' + SPEC.widgets.length + ' widgets and templates', value: S.search, 'aria-label': 'Search widgets and templates'});
    search.addEventListener('input', function () { S.search = search.value; filter(); });
    pane.appendChild(search);
    var sel = S.sel && locate(S.sel);
    if (sel) pane.appendChild(h('p', {class: 'vbe-hint', style: 'margin:10px 2px 0;font-size:12px;color:var(--vbe-ink-3)', text: 'Clicking adds ' + (sel.kind === 'widget' ? 'below the selected widget' : sel.kind === 'column' ? 'to the selected column' : 'to the selected section') + '. Sections and templates go after it.'}));
    // One list of every widget, A–Z: no Basic / Pro split to hunt across.
    var ws = SPEC.widgets.filter(function (w) {
      if (w.key === 'global' || w.cat === 'imported') return false;
      if (w.key === 'post-content') return IS_TEMPLATE && T.templateKind === 'single';
      return true;
    }).sort(function (a, b) { return a.label.localeCompare(b.label); });
    pane.appendChild(h('div', {class: 'vbe-h', 'data-cat': 'all', text: 'Widgets'}));
    var grid = h('div', {class: 'vbe-widgets', 'data-cat': 'all'});
    ws.forEach(function (w) {
      var tile = h('div', {class: 'vbe-wtile', draggable: 'true', tabindex: '0', role: 'button', 'data-find': (w.label + ' ' + w.key + ' ' + w.cat).toLowerCase(), title: 'Drag onto the page, or click to add', html: ui(w.key) + '<span>' + esc(w.label) + '</span>'});
      tile.addEventListener('dragstart', function (e) { DRAG = {kind: 'new', type: w.key}; e.dataTransfer.effectAllowed = 'copy'; try { e.dataTransfer.setData('text/plain', 'vb-new:' + w.key); } catch (x) { /* */ } });
      tile.addEventListener('dragend', endDrag);
      tile.addEventListener('click', function () { insertWidget(w.key); });
      tile.addEventListener('keydown', function (e) { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); insertWidget(w.key); } });
      grid.appendChild(tile);
    });
    pane.appendChild(grid);
    pane.appendChild(h('div', {class: 'vbe-h', text: 'Sections'}));
    var structs = h('div', {class: 'vbe-structs'});
    [[1], [1, 1], [1, 1, 1], [2, 1], [1, 2], [1, 1, 1, 1], [1, 2, 1], [3, 1], [1, 1, 1, 1, 1, 1]].forEach(function (ws) {
      var b = h('button', {class: 'vbe-struct', type: 'button', title: ws.length + ' column' + (ws.length > 1 ? 's' : '') + ' (' + ws.join(':') + ')', 'data-find': 'section columns layout'});
      ws.forEach(function (w) { b.appendChild(h('i', {style: 'flex:' + w})); });
      b.addEventListener('click', function () { insertSections([newSection(ws)]); });
      structs.appendChild(b);
    });
    pane.appendChild(structs);
    pane.appendChild(h('div', {class: 'vbe-h', text: 'Templates'}));
    var tpls = h('div', {class: 'vbe-tpls'});
    TEMPLATES.forEach(function (t) {
      var b = h('button', {class: 'vbe-tpl', type: 'button', 'data-find': (t.name + ' ' + t.find).toLowerCase(), html: '<span class="vbe-tpl-thumb">' + thumb(t.thumb) + '</span><b>' + esc(t.name) + '</b>'});
      b.addEventListener('click', function () { insertSections(t.make()); toast(t.name + ' added.'); });
      tpls.appendChild(b);
    });
    S.templates.filter(function (t) { return t.kind !== 'widget'; }).forEach(function (t) {
      var b = h('button', {class: 'vbe-tpl', type: 'button', 'data-find': (t.name + ' saved ' + t.kind).toLowerCase(), html: '<span class="vbe-tpl-thumb">' + thumb(t.kind === 'page' ? 'page' : 'saved') + '</span><b>' + esc(t.name) + '</b><small>Saved ' + (t.kind === 'page' ? 'page' : 'section') + '</small>'});
      b.addEventListener('click', function () {
        var list = (t.data && t.data.sections) || [];
        if (!list.length) { toast('That template is empty.', true); return; }
        insertSections(list);
        toast(t.name + ' added.');
      });
      tpls.appendChild(b);
    });
    pane.appendChild(tpls);
    var globals = S.templates.filter(function (t) { return t.kind === 'widget'; });
    if (globals.length) {
      pane.appendChild(h('div', {class: 'vbe-h', text: 'Global widgets'}));
      var gw = h('div', {class: 'vbe-widgets'});
      globals.forEach(function (t) {
        var tile = h('div', {class: 'vbe-wtile', tabindex: '0', role: 'button', 'data-find': (t.name + ' global').toLowerCase(), html: ui('save') + '<span>' + esc(t.name) + '</span>'});
        tile.addEventListener('click', function () { var w = insertWidget('global'); if (w) { w.s.ref = t.id; renderAll(); } });
        gw.appendChild(tile);
      });
      pane.appendChild(gw);
    }
    function filter() {
      var q = S.search.trim().toLowerCase();
      Array.prototype.forEach.call(pane.querySelectorAll('[data-find]'), function (n) { n.style.display = !q || n.dataset.find.indexOf(q) >= 0 ? '' : 'none'; });
      Array.prototype.forEach.call(pane.querySelectorAll('.vbe-widgets[data-cat]'), function (g) {
        var any = Array.prototype.some.call(g.children, function (c) { return c.style.display !== 'none'; });
        g.style.display = any ? '' : 'none';
        var head = pane.querySelector('.vbe-h[data-cat="' + g.dataset.cat + '"]');
        if (head) head.style.display = any ? '' : 'none';
      });
    }
    filter();
  }

  function fieldsFor(loc) {
    if (loc.kind === 'section') return SPEC.section;
    if (loc.kind === 'column') return SPEC.column;
    return (WSPEC[loc.w.t] || {fields: []}).fields;
  }

  function paneEdit(pane) {
    var loc = S.sel && locate(S.sel);
    if (!loc) {
      pane.appendChild(h('div', {class: 'vbe-empty-state', html: ui('edit') + '<p><b>Select something on the page</b></p><p>Click a widget, a column or a section to change it. Double-click text to type straight onto the page. Right-click for more.</p>'}));
      return;
    }
    var crumbs = h('div', {class: 'vbe-crumbs'});
    var chain = [['Section ' + (loc.si + 1), loc.sec.id]];
    var outer = loc.owner && loc.owner.t === 'inner' ? locate(loc.owner.id) : null;
    if (outer) { chain.push(['Column', outer.col.id]); chain.push(['Inner section', outer.w.id]); }
    if (loc.col && loc.kind !== 'section') chain.push(['Column ' + ((loc.col && (loc.owner && loc.owner.cols || loc.sec.cols).indexOf(loc.col)) + 1), loc.col.id]);
    if (loc.w) chain.push([label(loc.w.t), loc.w.id]);
    chain.forEach(function (c, i) {
      if (i) crumbs.appendChild(h('span', {text: '›'}));
      crumbs.appendChild(h('button', {class: c[1] === S.sel ? 'on' : '', text: c[0], onclick: function () { select(c[1]); }}));
    });
    pane.appendChild(crumbs);
    var kind = loc.kind, node = loc.node;
    var title = kind === 'section' ? 'Section' : kind === 'column' ? 'Column' : label(loc.w.t);
    pane.appendChild(h('div', {class: 'vbe-sel-head'}, [h('span', {class: 'vbe-ic', html: ui(kind === 'widget' ? loc.w.t : kind)}), h('b', {text: title}),
      h('button', {class: 'vbe-ib', title: 'Copy (⌘C)', html: ui('copy'), onclick: function () { copyNode(S.sel); }}),
      h('button', {class: 'vbe-ib', title: 'Duplicate (⌘D)', html: ui('copy'), onclick: function () { duplicate(S.sel); }}),
      h('button', {class: 'vbe-ib', title: 'Delete (Del)', html: ui('del'), onclick: function () { remove(S.sel); }})
    ]));
    var fields = fieldsFor(loc);
    var groups = [['c', kind === 'widget' ? 'Content' : 'Layout'], ['s', 'Style'], ['a', 'Advanced']].filter(function (g) { return fields.some(function (f) { return f.group === g[0]; }); });
    if (!groups.some(function (g) { return g[0] === S.subtab; })) S.subtab = groups[0][0];
    var sub = h('div', {class: 'vbe-subtabs'});
    groups.forEach(function (g) { sub.appendChild(h('button', {class: S.subtab === g[0] ? 'on' : '', text: g[1], onclick: function () { S.subtab = g[0]; renderSide(); }})); });
    pane.appendChild(sub);
    if (S.device !== 'desktop') pane.appendChild(h('p', {class: 'vbe-devnote', html: ui(S.device) + '<span>Editing <b>' + S.device + '</b>: settings marked with ' + ui(S.device) + ' apply to ' + S.device + ' only. Others are the same everywhere.</span>'}));
    var owner = kind === 'section' ? node : kind === 'widget' && node.t === 'inner' ? node : null;
    if (owner && S.subtab === 'c') {
      var f = h('div', {class: 'vbe-f'}, [h('div', {class: 'vbe-l', text: 'Columns'})]);
      var structs = h('div', {class: 'vbe-structs'});
      [[1], [1, 1], [1, 1, 1], [2, 1], [1, 2], [1, 1, 1, 1]].forEach(function (ws) {
        var b = h('button', {class: 'vbe-struct', type: 'button', title: ws.join(':')});
        ws.forEach(function (w) { b.appendChild(h('i', {style: 'flex:' + w})); });
        b.addEventListener('click', function () { setStructure(node.id, ws); });
        structs.appendChild(b);
      });
      f.appendChild(structs);
      pane.appendChild(f);
    }
    if (kind === 'widget' && (node.t === 'heading' || node.t === 'text') && S.subtab === 'c') {
      pane.appendChild(h('p', {style: 'margin:-4px 2px 12px;font-size:12px;color:var(--vbe-ink-3)', text: 'Tip: double-click the text on the page to edit it in place.'}));
    }
    if (kind === 'widget' && node.t === 'global' && S.subtab === 'c') {
      pane.appendChild(h('p', {style: 'margin:-4px 2px 12px;font-size:12px;color:var(--vbe-ink-3)', html: 'A linked copy. Change it under <a href="' + esc(BOOT.toolUrl) + 'templates/" target="_blank">Templates</a>, or right-click → Unlink to edit this one alone.'}));
    }
    node.s = node.s || {};
    fields.forEach(function (fd) {
      if (fd.group !== S.subtab) return;
      if (!visible(fd, node.s)) return;
      pane.appendChild(control(node.s, fd));
    });
    if (kind === 'section' && S.subtab === 'c') {
      pane.appendChild(h('div', {class: 'vbe-actions-row'}, [
        IS_TEMPLATE ? null : h('button', {class: 'vbe-btn tonal small', html: ui('save') + ' Save as template', onclick: function () { saveTemplate('section', {sections: [loc.sec]}); }}),
        h('button', {class: 'vbe-btn text small', html: ui('add') + ' Add column', onclick: function () { addColumn(loc.sec.id); }})
      ]));
    }
  }

  function visible(fd, obj) {
    if (!fd.when) return true;
    var v = obj[fd.when[0]];
    var sv = v === true ? 'true' : v === false || v == null ? '' : String(v);
    return fd.when[1].some(function (want) { return want === '*' ? sv !== '' : want === sv; });
  }

  // ---- one control, from a spec field ----
  function devKey(fd) { return fd.resp && S.device !== 'desktop' ? fd.key + (S.device === 'tablet' ? '_t' : '_m') : fd.key; }

  function control(obj, fd, opts) {
    opts = opts || {};
    var key = devKey(fd), kind = fd.kind;
    if (kind === 'heading') return h('div', {class: 'vbe-subhead', text: fd.label});
    var wrap = h('div', {class: 'vbe-f'});
    function set(v, coalesce) {
      record(coalesce ? (S.sel || 'page') + ':' + key : null);
      if (v === '' || v == null || v === false) delete obj[key]; else obj[key] = v;
      if (opts.onChange) opts.onChange(); else renderCanvasSoon();
      if (!coalesce && !opts.noRefresh) setTimeout(renderSide, 0);
      renderLayersSoon();
    }
    var inherited = fd.resp && key !== fd.key ? (obj[fd.key + (S.device === 'mobile' ? '_t' : '')] != null ? obj[fd.key + (S.device === 'mobile' ? '_t' : '')] : obj[fd.key]) : null;
    function lab(extra) {
      var l = h('label');
      var t = h('span', {text: fd.label});
      if (fd.resp) t.innerHTML = esc(fd.label) + ' <i class="vbe-resp" title="Can differ per device">' + ui(S.device) + '</i>';
      l.appendChild(t);
      if (extra) l.appendChild(extra);
      wrap.appendChild(l);
    }
    var v = obj[key];
    switch (kind) {
      case 'text':
      case 'ident':
      case 'url': {
        var tagBtn = (kind === 'text' || kind === 'url') && !opts.inItem ? tagPicker(kind === 'url', function (tag) { i.value = kind === 'url' ? tag : (i.value + tag); set(i.value); }) : null;
        lab(tagBtn);
        var i = h('input', {class: 'vbe-in', type: 'text', value: v == null ? '' : v, 'data-fkey': key, maxlength: fd.max || null, placeholder: inherited != null ? String(inherited) : (kind === 'url' ? 'https://…, /page/, #anchor or #/portal/signup' : '')});
        i.addEventListener('input', function () { set(i.value, true); });
        i.addEventListener('change', function () { lastKey = null; });
        wrap.appendChild(i);
        break;
      }
      case 'long':
      case 'code': {
        lab(kind === 'long' && !opts.inItem ? tagPicker(false, function (tag) { t.value += tag; set(t.value); }) : null);
        var t = h('textarea', {class: 'vbe-ta' + (kind === 'code' ? ' code' : ''), rows: kind === 'code' ? 8 : 3, spellcheck: kind === 'code' ? 'false' : 'true', 'data-fkey': key});
        t.value = v || '';
        t.addEventListener('input', function () { set(t.value, true); });
        wrap.appendChild(t);
        break;
      }
      case 'rich': {
        lab(tagPicker(false, function (tag) { body.focus(); document.execCommand('insertText', false, tag); set(body.innerHTML, true); }));
        var box = h('div', {class: 'vbe-rich'});
        var bar = h('div', {class: 'vbe-rich-bar'});
        var body = h('div', {class: 'vbe-rich-body', contenteditable: 'true'});
        body.innerHTML = v || '';
        RICH.forEach(function (c) {
          bar.appendChild(h('button', {class: 'vbe-ib', type: 'button', text: c[1], title: c[2], onmousedown: function (e) { e.preventDefault(); richCommand(document, c[0], function () { set(body.innerHTML, true); }); }}));
        });
        body.addEventListener('input', function () { set(body.innerHTML, true); });
        box.appendChild(bar); box.appendChild(body);
        wrap.appendChild(box);
        break;
      }
      case 'choice': {
        var o = fd.options;
        var alignIcons = {left: 'al', center: 'ac', right: 'ar', justify: 'aj', full: 'aj'};
        var isAlign = /align$/.test(fd.key) && o.every(function (x) { return alignIcons[x[0]] || x[0] === ''; });
        var cur = v == null ? '' : String(v);
        var short = o.length <= 5 && o.every(function (x) { return x[1].length <= 9; });
        lab();
        if (isAlign || short) {
          var seg = h('div', {class: 'vbe-seg', role: 'group', 'aria-label': fd.label});
          o.forEach(function (x, n) {
            var on = cur === x[0] || (!cur && n === 0 && inherited == null) || (!cur && inherited != null && String(inherited) === x[0]);
            var b = h('button', {type: 'button', class: on ? 'on' + (cur ? '' : ' inherit') : '', title: x[1], 'aria-pressed': on ? 'true' : 'false', html: isAlign && alignIcons[x[0]] ? ui(alignIcons[x[0]]) : esc(x[1])});
            b.addEventListener('click', function () { set(x[0] === cur && key !== fd.key ? null : x[0]); });
            seg.appendChild(b);
          });
          wrap.appendChild(seg);
        } else {
          var s = h('select', {class: 'vbe-sel'});
          if (key !== fd.key) s.appendChild(h('option', {value: '', text: 'Same as ' + (S.device === 'mobile' ? 'tablet' : 'desktop')}));
          o.forEach(function (x) { s.appendChild(h('option', {value: x[0], text: x[1], selected: cur === x[0]})); });
          s.addEventListener('change', function () { set(s.value); });
          wrap.appendChild(s);
        }
        break;
      }
      case 'font': {
        lab();
        var fs = h('select', {class: 'vbe-sel'});
        fs.appendChild(h('option', {value: '', text: 'Default'}));
        SPEC.fonts.forEach(function (x) { fs.appendChild(h('option', {value: x[0], text: x[1], selected: v === x[0]})); });
        fs.addEventListener('change', function () { set(fs.value); });
        wrap.appendChild(fs);
        break;
      }
      case 'catalogue': {
        lab();
        var selected = Array.isArray(v) ? v : [];
        wrap.appendChild(h('button', {class: 'vbe-btn tonal small', type: 'button',
          text: (selected.length ? 'Edit selection (' + selected.length + ')' : 'Choose ' + fd.label.toLowerCase()),
          onclick: function () {
            cataloguePicker(fd, selected).then(function (ids) {
              if (ids && ids.join(',') !== selected.join(',')) set(ids);
            });
          }}));
        break;
      }
      case 'template':
      case 'form': {
        lab();
        var ts = h('select', {class: 'vbe-sel'});
        ts.appendChild(h('option', {value: '', text: 'Choose…'}));
        var list = kind === 'form' ? BOOT.forms : S.refs.filter(function (r) { return fd.of === 'widget' ? r[2] === 'widget' : (r[2] === 'section' || r[2] === 'page'); });
        list.forEach(function (x) { ts.appendChild(h('option', {value: x[0], text: x[1], selected: v === x[0]})); });
        ts.addEventListener('change', function () { set(ts.value); });
        wrap.appendChild(ts);
        if (!list.length) wrap.appendChild(h('div', {class: 'vbe-hint', text: kind === 'form' ? 'No forms yet — create one under Tools → Forms.' : 'Nothing saved yet.'}));
        break;
      }
      case 'bool': {
        var l = h('label', {style: 'cursor:pointer'});
        l.appendChild(h('span', {text: fd.label}));
        l.appendChild(h('span', {class: 'vbe-switch'}, [h('input', {type: 'checkbox', checked: !!v, onchange: function (e) { set(e.target.checked); }}), h('span')]));
        wrap.appendChild(l);
        break;
      }
      case 'num': {
        var shown = h('span', {style: 'font-weight:400;color:var(--vbe-ink-3)', text: v == null ? (inherited != null ? inherited + (fd.unit || '') + ' (inherited)' : 'Default') : v + (fd.unit || '')});
        var reset = v != null ? h('button', {class: 'vbe-btn text small', style: 'height:22px;padding:0 6px', text: 'Reset', onclick: function () { set(null); }}) : null;
        lab(h('span', {style: 'display:flex;gap:6px;align-items:center'}, [shown, reset]));
        var rg = h('div', {class: 'vbe-range'});
        var start = v != null ? v : inherited != null ? inherited : Math.max(fd.min, 0);
        var r = h('input', {type: 'range', min: fd.min, max: Math.min(fd.max, fd.unit === 'px' && fd.max > 400 ? 400 : fd.max), step: 1, value: start});
        var n = h('input', {class: 'vbe-in', type: 'number', min: fd.min, max: fd.max, value: v != null ? v : '', placeholder: inherited != null ? String(inherited) : '', 'data-fkey': key});
        r.addEventListener('input', function () { n.value = r.value; shown.textContent = r.value + (fd.unit || ''); set(Number(r.value), true); });
        r.addEventListener('change', function () { lastKey = null; renderSide(); });
        n.addEventListener('change', function () { if (n.value === '') { set(null); return; } set(Math.max(fd.min, Math.min(fd.max, Number(n.value)))); });
        rg.appendChild(r); rg.appendChild(n);
        wrap.appendChild(rg);
        break;
      }
      case 'sides': {
        lab(v != null ? h('button', {class: 'vbe-btn text small', style: 'height:22px;padding:0 6px', text: 'Reset', onclick: function () { set(null); }}) : null);
        var parts = String(v || inherited || '').split(' ');
        var row = h('div', {class: 'vbe-sides'});
        var linked = parts.length < 2 || parts.every(function (p) { return p === parts[0]; });
        var ins = ['Top', 'Right', 'Bottom', 'Left'].map(function (nm, k) {
          var inp = h('input', {class: 'vbe-in', type: 'number', min: fd.min, max: fd.max, placeholder: nm.charAt(0), title: nm, value: v != null ? (parts[k] || parts[0] || '') : ''});
          inp.addEventListener('input', function () {
            if (linkBtn.classList.contains('on')) ins.forEach(function (o2) { if (o2 !== inp) o2.value = inp.value; });
            var vals = ins.map(function (x) { return x.value === '' ? '0' : String(Math.max(fd.min, Math.min(fd.max, Number(x.value)))); });
            set(ins.every(function (x) { return x.value === ''; }) ? null : vals.join(' '), true);
          });
          row.appendChild(h('label', {class: 'vbe-sidein'}, [inp, h('small', {text: nm})]));
          return inp;
        });
        var linkBtn = h('button', {type: 'button', class: 'vbe-ib vbe-link' + (linked ? ' on' : ''), title: 'Same value on every side', html: ui('link'), onclick: function () { linkBtn.classList.toggle('on'); }});
        row.appendChild(linkBtn);
        wrap.appendChild(row);
        break;
      }
      case 'date': {
        lab();
        var di = h('input', {class: 'vbe-in', type: 'datetime-local', value: v || ''});
        di.addEventListener('change', function () { set(di.value); });
        wrap.appendChild(di);
        break;
      }
      case 'colour': {
        lab();
        var cr = h('div', {class: 'vbe-colours'});
        cr.appendChild(h('button', {type: 'button', class: 'vbe-sw none' + (!v ? ' on' : ''), title: 'Default', onclick: function () { set(null); }}));
        BOOT.settings.palette.forEach(function (p) {
          cr.appendChild(h('button', {type: 'button', class: 'vbe-sw' + (v === p.key ? ' on' : ''), title: p.name + ' (global)', style: 'background:' + p.value, onclick: function () { set(p.key); }}));
        });
        var custom = h('label', {class: 'vbe-sw custom' + (v && v.charAt(0) === '#' ? ' on' : ''), title: 'Custom colour'});
        if (v && v.charAt(0) === '#') custom.style.background = v;
        var ci = h('input', {type: 'color', value: v && v.charAt(0) === '#' && v.length === 7 ? v : '#ff1a75'});
        ci.addEventListener('input', function () { custom.style.background = ci.value; set(ci.value, true); });
        ci.addEventListener('change', function () { lastKey = null; renderSide(); });
        custom.appendChild(ci);
        cr.appendChild(custom);
        var hex = h('input', {class: 'vbe-in', style: 'width:96px;min-height:30px;padding:4px 8px;font:12px var(--vbe-mono)', value: v && v.charAt(0) === '#' ? v : '', placeholder: '#hex'});
        hex.addEventListener('change', function () { var x = hex.value.trim(); if (!x) set(null); else if (/^#([0-9a-f]{3}|[0-9a-f]{6}|[0-9a-f]{8})$/i.test(x)) set(x.toLowerCase()); else toast('Use a colour like #1a2b3c.', true); });
        cr.appendChild(hex);
        wrap.appendChild(cr);
        break;
      }
      case 'image': {
        lab();
        wrap.appendChild(imagePicker(v, function (url) { set(url); }));
        break;
      }
      case 'icon': {
        lab();
        var grid = h('div', {class: 'vbe-icons'});
        Object.keys(ICONS).forEach(function (name) {
          grid.appendChild(h('button', {type: 'button', class: v === name ? 'on' : '', title: name, html: iconSvg(name), onclick: function () { set(name); }}));
        });
        wrap.appendChild(grid);
        var em = h('input', {class: 'vbe-in', style: 'margin-top:8px', maxlength: '8', placeholder: 'Or type an emoji, e.g. 🚀', value: v && !ICONS[v] ? v : ''});
        em.addEventListener('input', function () { set(em.value.trim(), true); });
        wrap.appendChild(em);
        break;
      }
      case 'items': {
        lab();
        wrap.appendChild(key === 'images' ? galleryEditor(obj, key) : itemsEditor(obj, key, fd));
        break;
      }
    }
    if (fd.hint) wrap.appendChild(h('div', {class: 'vbe-hint', text: fd.hint}));
    return wrap;
  }

  /* [[tag]] picker for text fields. */
  function tagPicker(urls, insert) {
    var tags = urls ? [['post.url', 'Post URL'], ['post.feature_image', 'Feature image'], ['post.author_url', 'Author page'], ['site.url', 'Site URL'], ['site.logo', 'Site logo']] : SPEC.dynamicTags;
    var b = h('button', {type: 'button', class: 'vbe-tagbtn', title: 'Insert a dynamic tag', text: '[[ ]]'});
    b.addEventListener('click', function (e) {
      e.preventDefault();
      hideMenu();
      menuEl = h('div', {class: 'vbe-menu', role: 'menu'});
      menuEl.appendChild(h('div', {class: 'vbe-menu-head', text: 'Dynamic tags'}));
      tags.forEach(function (t) {
        menuEl.appendChild(h('button', {type: 'button', role: 'menuitem', html: '<span>' + esc(t[1]) + '</span><kbd>' + esc(t[0]) + '</kbd>', onclick: function () { hideMenu(); insert('[[' + t[0] + ']]'); }}));
      });
      document.body.appendChild(menuEl);
      var r = b.getBoundingClientRect();
      menuEl.style.left = Math.max(8, r.right - menuEl.offsetWidth) + 'px';
      menuEl.style.top = Math.min(r.bottom + 4, innerHeight - menuEl.offsetHeight - 8) + 'px';
    });
    return b;
  }

  function imagePicker(value, onPick) {
    var box = h('div', {class: 'vbe-img-pick'});
    var prev = h('div', {class: 'vbe-img-prev', text: value ? (value.indexOf('[[') === 0 ? value : '') : 'No image'});
    if (value && value.indexOf('[[') !== 0) prev.style.backgroundImage = "url('" + value.replace(/'/g, '%27') + "')";
    box.appendChild(prev);
    var url = h('input', {class: 'vbe-in', value: value || '', placeholder: 'Image address, or upload one'});
    url.addEventListener('change', function () { onPick(url.value.trim()); });
    var file = h('input', {type: 'file', accept: 'image/*', style: 'display:none'});
    file.addEventListener('change', function () {
      if (!file.files.length) return;
      prev.textContent = 'Uploading…';
      upload(file.files[0]).then(function (u) { onPick(u); toast('Image uploaded.'); }).catch(function (e) { prev.textContent = 'Upload failed'; toast(e.message, true); });
    });
    box.appendChild(h('div', {class: 'vbe-row-btns'}, [
      h('button', {class: 'vbe-btn tonal small', type: 'button', html: ui('upload') + ' Upload', onclick: function () { file.click(); }}),
      BOOT.unsplash && window.GhostUnsplash ? h('button', {class: 'vbe-btn text small', type: 'button', html: ui('image') + ' Unsplash', onclick: function () { window.GhostUnsplash.open({headers: BOOT.unsplash, onSelect: function (src) { onPick(src); }}); }}) : null,
      value ? h('button', {class: 'vbe-btn text small danger', type: 'button', text: 'Remove', onclick: function () { onPick(''); }}) : null
    ]));
    box.appendChild(url);
    box.appendChild(file);
    return box;
  }

  function upload(fileObj) {
    var form = new FormData();
    form.append('file', fileObj);
    return fetch(BOOT.upload, {method: 'POST', body: form, credentials: 'same-origin'}).then(function (r) {
      return r.json().then(function (j) {
        if (!r.ok) throw new Error((j.errors && j.errors[0] && j.errors[0].message) || 'The upload was refused.');
        return j.images[0].url;
      });
    });
  }

  var openItem = {};
  function itemsEditor(obj, key, fd) {
    var list = obj[key] = obj[key] || [];
    var box = h('div', {class: 'vbe-items'});
    var first = fd.fields.find(function (f) { return f.kind === 'text' || f.kind === 'choice' || f.kind === 'ident'; });
    list.forEach(function (it, i) {
      var k = (S.sel || 'p') + key + i;
      var item = h('div', {class: 'vbe-item' + (openItem[k] ? ' open' : '')});
      var head = h('div', {class: 'vbe-item-head'}, [
        h('span', {text: String((first && it[first.key]) || 'Item ' + (i + 1))}),
        h('button', {class: 'vbe-ib', type: 'button', title: 'Move up', html: ui('up'), disabled: i === 0, onclick: function (e) { e.stopPropagation(); record(); list.splice(i - 1, 0, list.splice(i, 1)[0]); renderCanvasSoon(); renderSide(); }}),
        h('button', {class: 'vbe-ib', type: 'button', title: 'Move down', html: ui('down'), disabled: i === list.length - 1, onclick: function (e) { e.stopPropagation(); record(); list.splice(i + 1, 0, list.splice(i, 1)[0]); renderCanvasSoon(); renderSide(); }}),
        h('button', {class: 'vbe-ib', type: 'button', title: 'Duplicate', html: ui('copy'), onclick: function (e) { e.stopPropagation(); record(); list.splice(i + 1, 0, clone(it)); renderCanvasSoon(); renderSide(); }}),
        h('button', {class: 'vbe-ib', type: 'button', title: 'Remove', html: ui('del'), onclick: function (e) { e.stopPropagation(); record(); list.splice(i, 1); renderCanvasSoon(); renderSide(); }})
      ]);
      head.addEventListener('click', function () { openItem[k] = !openItem[k]; item.classList.toggle('open'); });
      item.appendChild(head);
      var body = h('div', {class: 'vbe-item-body'});
      fd.fields.forEach(function (f) { body.appendChild(control(it, f, {noRefresh: true, inItem: true})); });
      item.appendChild(body);
      box.appendChild(item);
    });
    if (list.length < (fd.max || 60)) box.appendChild(h('button', {class: 'vbe-btn tonal small', type: 'button', html: ui('add') + ' Add', onclick: function () {
      record();
      var blank = {};
      fd.fields.forEach(function (f) { if (f.kind === 'text') blank[f.key] = 'New item'; if (f.kind === 'choice') blank[f.key] = f.options[0][0]; if (f.kind === 'num') blank[f.key] = Math.max(f.min, Math.min(f.max, 50)); });
      list.push(blank);
      openItem[(S.sel || 'p') + key + (list.length - 1)] = true;
      renderCanvasSoon(); renderSide();
    }}));
    return box;
  }

  function galleryEditor(obj, key) {
    var list = obj[key] = obj[key] || [];
    var box = h('div', {class: 'vbe-items'});
    var grid = h('div', {style: 'display:grid;grid-template-columns:repeat(3,1fr);gap:6px'});
    list.forEach(function (it, i) {
      var cell = h('div', {style: "position:relative;aspect-ratio:1;border-radius:8px;background:#eee center/cover;background-image:url('" + String(it.src || '').replace(/'/g, '%27') + "')"});
      cell.appendChild(h('button', {class: 'vbe-ib', type: 'button', title: 'Remove', style: 'position:absolute;top:2px;right:2px;width:24px;height:24px;background:rgba(255,255,255,.9)', html: ui('close'), onclick: function () { record(); list.splice(i, 1); renderCanvasSoon(); renderSide(); }}));
      grid.appendChild(cell);
    });
    box.appendChild(grid);
    var file = h('input', {type: 'file', accept: 'image/*', multiple: true, style: 'display:none'});
    file.addEventListener('change', function () {
      var files = Array.prototype.slice.call(file.files);
      if (!files.length) return;
      toast('Uploading ' + files.length + ' image' + (files.length > 1 ? 's' : '') + '…');
      Promise.all(files.map(upload)).then(function (urls) {
        record();
        urls.forEach(function (u) { list.push({src: u, alt: ''}); });
        renderCanvasSoon(); renderSide();
        toast('Added.');
      }).catch(function (e) { toast(e.message, true); });
    });
    box.appendChild(file);
    box.appendChild(h('div', {class: 'vbe-row-btns'}, [
      h('button', {class: 'vbe-btn tonal small', type: 'button', html: ui('upload') + ' Upload images', onclick: function () { file.click(); }}),
      h('button', {class: 'vbe-btn text small', type: 'button', html: ui('link') + ' Add by address', onclick: function () {
        var u = window.prompt('Image address', 'https://');
        if (u && /^(https?:\/\/|\/)/.test(u)) { record(); list.push({src: u, alt: ''}); renderCanvasSoon(); renderSide(); }
      }})
    ]));
    return box;
  }

  // ---- layers ----
  var layersPending = false;
  function renderLayersSoon() {
    if (S.tab !== 'layers' || layersPending) return;
    layersPending = true;
    setTimeout(function () { layersPending = false; if (S.tab === 'layers') renderSide(); }, 200);
  }
  function paneLayers(pane) {
    if (!sections().length) { pane.appendChild(h('div', {class: 'vbe-empty-state', html: ui('layers') + '<p>The page is empty.</p>'})); return; }
    var tree = h('div', {class: 'vbe-layers'});
    var LDRAG = null;
    function clearMarks() { Array.prototype.forEach.call(tree.querySelectorAll('.drop-before,.drop-after'), function (n) { n.classList.remove('drop-before', 'drop-after'); }); }
    function row(depth, icon, text, id, kind) {
      var r = h('div', {class: 'vbe-layer' + (S.sel === id ? ' on' : ''), 'data-depth': depth, draggable: kind !== 'column' ? 'true' : null});
      r.style.paddingLeft = (6 + depth * 16) + 'px';
      r.innerHTML = ui(icon) + '<span>' + esc(text) + '</span>';
      r.appendChild(h('button', {class: 'vbe-ib', title: 'Duplicate', html: ui('copy'), onclick: function (e) { e.stopPropagation(); duplicate(id); }}));
      r.appendChild(h('button', {class: 'vbe-ib', title: 'Delete', html: ui('del'), onclick: function (e) { e.stopPropagation(); remove(id); }}));
      r.addEventListener('click', function () { select(id, false); S.tab = 'layers'; renderSide(); scrollIntoView(id); });
      r.addEventListener('dblclick', function () { select(id); });
      r.addEventListener('contextmenu', function (e) { e.preventDefault(); select(id, false); showMenu(id, e.clientX, e.clientY); });
      r.addEventListener('dragstart', function (e) { LDRAG = {id: id, kind: kind}; e.dataTransfer.effectAllowed = 'move'; try { e.dataTransfer.setData('text/plain', id); } catch (x) { /* */ } });
      r.addEventListener('dragover', function (e) {
        if (!LDRAG) return;
        var ok = (LDRAG.kind === 'section' && kind === 'section') || (LDRAG.kind === 'widget' && (kind === 'widget' || kind === 'column'));
        if (!ok) return;
        e.preventDefault();
        var rc = r.getBoundingClientRect(), after = e.clientY > rc.top + rc.height / 2 || kind === 'column';
        clearMarks();
        r.classList.add(after ? 'drop-after' : 'drop-before');
      });
      r.addEventListener('drop', function (e) {
        if (!LDRAG) return;
        e.preventDefault();
        var after = r.classList.contains('drop-after');
        var d = LDRAG; LDRAG = null;
        clearMarks();
        if (d.id === id) return;
        if (d.kind === 'section') {
          record();
          var from = locate(d.id);
          var s = sections().splice(from.index, 1)[0];
          sections().splice(locate(id).index + (after ? 1 : 0), 0, s);
          renderAll();
        } else if (kind === 'column') moveWidget(d.id, id, locate(id).col.items.length);
        else { var t = locate(id); moveWidget(d.id, t.col.id, t.index + (after ? 1 : 0)); }
      });
      r.addEventListener('dragend', function () { LDRAG = null; clearMarks(); });
      tree.appendChild(r);
    }
    function cols(list, depth) {
      list.forEach(function (col, ci) {
        if (list.length > 1 || !col.items.length) row(depth, 'column', 'Column ' + (ci + 1), col.id, 'column');
        var d = list.length > 1 || !col.items.length ? depth + 1 : depth;
        col.items.forEach(function (w) {
          var txt = w.s && (w.s.text || w.s.title || w.s.name || w.s.before);
          row(d, w.t, label(w.t) + (txt ? ' — ' + String(txt).slice(0, 40) : ''), w.id, 'widget');
          if (w.t === 'inner' && w.cols) cols(w.cols, d + 1);
        });
      });
    }
    sections().forEach(function (sec, si) {
      row(0, 'section', 'Section ' + (si + 1) + (sec.s && sec.s.anchor ? ' #' + sec.s.anchor : ''), sec.id, 'section');
      cols(sec.cols, 1);
    });
    pane.appendChild(h('p', {style: 'margin:0 2px 10px;font-size:12px;color:var(--vbe-ink-3)', text: 'Drag to reorder. Double-click to edit. Right-click for more.'}));
    pane.appendChild(tree);
  }

  // ---- page / template settings ----
  function panePage(pane) {
    var p = BOOT.post;
    S.layout.page = S.layout.page || {};
    if (IS_TEMPLATE) {
      pane.appendChild(h('div', {class: 'vbe-h', text: T.kindLabel}));
      var nm = h('div', {class: 'vbe-f'}, [h('label', {text: 'Name'})]);
      var ni = h('input', {class: 'vbe-in', value: S.name, maxlength: '120'});
      ni.addEventListener('input', function () { S.name = ni.value; dirty(true); });
      nm.appendChild(ni);
      pane.appendChild(nm);
      if (T.theme) themeOptions(pane);
    } else {
      pane.appendChild(h('div', {class: 'vbe-h', text: 'On the site'}));
      pane.appendChild(h('div', {class: 'vbe-f'}, [h('label', {style: 'cursor:pointer'}, [h('span', {text: 'Show this layout to visitors'}), h('span', {class: 'vbe-switch'}, [h('input', {type: 'checkbox', checked: S.active, onchange: function (e) { S.active = e.target.checked; dirty(true); renderTop(); }}), h('span')])]),
        h('div', {class: 'vbe-hint', text: 'Off: the page shows what was written in the standard editor. The layout is kept.'})]));
      if (p.type === 'page') {
        pane.appendChild(h('div', {class: 'vbe-f'}, [h('label', {style: 'cursor:pointer'}, [h('span', {text: 'Theme title and feature image'}), h('span', {class: 'vbe-switch'}, [h('input', {type: 'checkbox', checked: S.showTitle, onchange: function (e) { S.showTitle = e.target.checked; dirty(true); }}), h('span')])]),
          h('div', {class: 'vbe-hint', text: 'Most builder pages bring their own heading: turn this off for a clean canvas. Takes effect when you save.'})]));
      }
      pane.appendChild(h('div', {class: 'vbe-hint', style: 'margin:-4px 2px 6px', html: 'Status: <b>' + esc(p.status) + '</b>. Publish, schedule and set the URL in <a href="' + esc(p.editUrl) + '">the standard editor</a>.'}));
    }
    pane.appendChild(h('div', {class: 'vbe-h', text: 'Style'}));
    SPEC.page.forEach(function (f) { pane.appendChild(control(S.layout.page, f)); });
    pane.appendChild(h('div', {class: 'vbe-h', text: 'Templates and data'}));
    pane.appendChild(h('div', {class: 'vbe-actions-row'}, [
      IS_TEMPLATE ? null : h('button', {class: 'vbe-btn tonal small', html: ui('save') + ' Save page as template', onclick: function () { saveTemplate('page', S.layout); }}),
      h('button', {class: 'vbe-btn text small', html: ui('copy') + ' Export', onclick: exportJson}),
      h('button', {class: 'vbe-btn text small', html: ui('upload') + ' Import', onclick: importJson}),
      h('button', {class: 'vbe-btn text small danger', html: ui('del') + ' Clear', onclick: function () {
        dialog('Clear everything?', 'Every section is removed. You can undo this, and nothing changes on the site until you save.', null, 'Clear').then(function (ok) { if (!ok) return; record(); S.layout.sections = []; S.sel = null; renderAll(); });
      }})
    ]));
    if (BOOT.endpoints.revisions) {
      pane.appendChild(h('div', {class: 'vbe-h', text: 'History'}));
      var revs = h('div', {class: 'vbe-revs', html: '<p style="margin:0;font-size:12.5px;color:var(--vbe-ink-3)">Loading earlier versions…</p>'});
      pane.appendChild(revs);
      fetch(BOOT.endpoints.revisions, {credentials: 'same-origin', headers: {Accept: 'application/json'}}).then(function (r) { return r.json(); }).then(function (j) {
        revs.innerHTML = '';
        if (!j.revisions || !j.revisions.length) { revs.appendChild(h('p', {style: 'margin:0;font-size:12.5px;color:var(--vbe-ink-3)', text: 'Every save keeps the version before it — the last 30 are listed here.'})); return; }
        j.revisions.forEach(function (rv) {
          revs.appendChild(h('div', {class: 'vbe-rev'}, [
            h('div', null, [h('b', {text: when(rv.at)}), h('small', {text: rv.by ? 'by ' + rv.by : 'earlier version'})]),
            h('button', {class: 'vbe-btn text small', text: 'Restore', onclick: function () {
              fetch(BOOT.endpoints.revisions + encodeURIComponent(rv.id) + '/', {credentials: 'same-origin', headers: {Accept: 'application/json'}}).then(function (r) { return r.json(); }).then(function (k) {
                if (!k.layout) throw new Error(k.error || 'That version could not be read.');
                record(); S.layout = k.layout; S.sel = null; renderAll();
                toast('Version from ' + when(rv.at) + ' restored. Save to keep it.');
              }).catch(function (e) { toast(e.message, true); });
            }})
          ]));
        });
      }).catch(function () { revs.textContent = 'History could not be loaded.'; });
    }
    pane.appendChild(h('div', {class: 'vbe-h', text: 'Shortcuts'}));
    pane.appendChild(h('div', {class: 'vbe-kbd', html: '<kbd>⌘S</kbd><span>Save</span><kbd>⌘Z / ⇧⌘Z</kbd><span>Undo / redo</span><kbd>⌘K</kbd><span>Finder</span><kbd>⌘C / ⌘V</kbd><span>Copy / paste element</span><kbd>⇧⌘V</kbd><span>Paste style</span><kbd>⌘D</kbd><span>Duplicate</span><kbd>Delete</kbd><span>Delete</span><kbd>Alt ↑ / ↓</kbd><span>Move</span><kbd>Esc</kbd><span>Select the parent</span><kbd>Right-click</kbd><span>More actions</span>'}));
  }

  function themeOptions(pane) {
    var o = S.options;
    function choice(key, lbl, opts2) {
      var f = h('div', {class: 'vbe-f'}, [h('label', {text: lbl})]);
      var s = h('select', {class: 'vbe-sel'});
      opts2.forEach(function (x) { s.appendChild(h('option', {value: x[0], text: x[1], selected: String(o[key]) === x[0]})); });
      s.addEventListener('change', function () { o[key] = s.value; dirty(true); renderSide(); });
      f.appendChild(s);
      return f;
    }
    function text(key, lbl, hint, area) {
      var f = h('div', {class: 'vbe-f'}, [h('label', {text: lbl})]);
      var i = area ? h('textarea', {class: 'vbe-ta', rows: 3}) : h('input', {class: 'vbe-in'});
      i.value = o[key] == null ? '' : o[key];
      i.addEventListener('input', function () { o[key] = i.value; dirty(true); });
      f.appendChild(i);
      if (hint) f.appendChild(h('div', {class: 'vbe-hint', text: hint}));
      return f;
    }
    function bool(key, lbl) {
      return h('div', {class: 'vbe-f'}, [h('label', {style: 'cursor:pointer'}, [h('span', {text: lbl}), h('span', {class: 'vbe-switch'}, [h('input', {type: 'checkbox', checked: !!o[key], onchange: function (e) { o[key] = e.target.checked; dirty(true); }}), h('span')])])]);
    }
    pane.appendChild(h('div', {class: 'vbe-f'}, [h('label', {style: 'cursor:pointer'}, [h('span', {text: 'Switched on'}), h('span', {class: 'vbe-switch'}, [h('input', {type: 'checkbox', checked: S.active, onchange: function (e) { S.active = e.target.checked; dirty(true); renderTop(); }}), h('span')])]), h('div', {class: 'vbe-hint', text: 'Off: nothing shows on the site. Saved changes are kept either way.'})]));
    pane.appendChild(h('div', {class: 'vbe-h', text: 'Display conditions'}));
    if (T.templateKind === 'single') {
      pane.appendChild(choice('post_types', 'Use for', [['post', 'Every post'], ['page', 'Every page'], ['all', 'Posts and pages']]));
      pane.appendChild(text('tags', 'Only with these tags', 'Tag slugs, separated by commas. Empty: any tag. Posts with their own layout keep it.'));
    } else {
      pane.appendChild(choice('include', 'Show on', [['all', 'The entire site'], ['home', 'The homepage'], ['paths', 'Specific addresses']]));
      if (o.include === 'paths') pane.appendChild(text('paths', 'Addresses', 'One per line, e.g. /about/ — end with * for everything under it: /blog/*', true));
    }
    pane.appendChild(text('exclude', 'Except', 'Addresses to leave out, one per line.', true));
    pane.appendChild(choice('who', 'Show to', [['any', 'Everyone'], ['visitors', 'Signed-out visitors'], ['members', 'Any member'], ['free', 'Free members'], ['paid', 'Paid members']]));
    pane.appendChild(text('priority', 'Priority', 'When two apply to one page, the higher number wins.'));
    if (T.templateKind === 'popup') {
      pane.appendChild(h('div', {class: 'vbe-h', text: 'Popup'}));
      pane.appendChild(choice('trigger', 'Opens', T.triggers));
      if (o.trigger === 'load' || o.trigger === 'scroll' || o.trigger === 'idle') pane.appendChild(text('value', o.trigger === 'scroll' ? 'After scrolling (%)' : 'After (seconds)'));
      pane.appendChild(choice('freq', 'How often', T.freqs));
      pane.appendChild(choice('position', 'Position', T.positions));
      pane.appendChild(text('width', 'Width (px)'));
      pane.appendChild(text('radius', 'Rounded corners (px)'));
      pane.appendChild(bool('overlay', 'Dim the page behind'));
      pane.appendChild(bool('overlay_close', 'Close when the dimmed page is clicked'));
      pane.appendChild(h('div', {class: 'vbe-hint', text: 'Any link to #popup-' + T.id + ' also opens it — set a button\'s link to that.'}));
    }
  }

  function when(at) {
    if (!at) return '';
    var d = new Date(at.replace(' ', 'T') + (/[zZ]|[+-]\d\d:?\d\d$/.test(at) ? '' : 'Z'));
    return isNaN(d) ? at : d.toLocaleString(undefined, {dateStyle: 'medium', timeStyle: 'short'});
  }

  // ---------------------------------------------------------------------
  // Finder (⌘K)
  // ---------------------------------------------------------------------
  function finder() {
    var input = h('input', {class: 'vbe-in', placeholder: 'Add a widget, jump to a section, switch device…', 'aria-label': 'Finder'});
    var list = h('div', {class: 'vbe-finder-list', role: 'listbox'});
    var box = h('div', null, [input, list]);
    var cmds = [];
    SPEC.widgets.forEach(function (w) { if (w.key !== 'global' && w.cat !== 'imported') cmds.push(['Add ' + w.label, w.key, function () { insertWidget(w.key); }]); });
    TEMPLATES.forEach(function (t) { cmds.push(['Insert template: ' + t.name, 'layout', function () { insertSections(t.make()); }]); });
    sections().forEach(function (s, i) { cmds.push(['Go to section ' + (i + 1) + (s.s && s.s.anchor ? ' #' + s.s.anchor : ''), 'section', function () { select(s.id); scrollIntoView(s.id); }]); });
    [['desktop', 'Desktop'], ['tablet', 'Tablet'], ['mobile', 'Mobile']].forEach(function (d) { cmds.push(['Preview on ' + d[1], d[0], function () { setDevice(d[0]); }]); });
    cmds.push(['Save', 'save', save]);
    if (!IS_TEMPLATE) cmds.push(['Preview in a new tab', 'eye', preview]);
    cmds.push(['Navigator (layers)', 'layers', function () { S.tab = 'layers'; renderSide(); }]);
    cmds.push([IS_TEMPLATE ? 'Template settings' : 'Page settings', 'page', function () { S.tab = 'page'; renderSide(); }]);
    cmds.push(['Builder settings (global colours, fonts…)', 'page', function () { window.open(BOOT.toolUrl + 'settings/', '_blank'); }]);
    cmds.push(['Theme Builder', 'page', function () { window.open(BOOT.toolUrl + 'theme/', '_blank'); }]);
    var shown = [], idx = 0, dlg;
    function draw() {
      var q = input.value.trim().toLowerCase();
      shown = cmds.filter(function (c) { return !q || c[0].toLowerCase().indexOf(q) >= 0; }).slice(0, 40);
      idx = Math.min(idx, Math.max(0, shown.length - 1));
      list.innerHTML = '';
      shown.forEach(function (c, i) {
        list.appendChild(h('button', {type: 'button', role: 'option', class: 'vbe-finder-item' + (i === idx ? ' on' : ''), html: ui(c[1]) + '<span>' + esc(c[0]) + '</span>', onclick: function () { run(i); }}));
      });
      if (!shown.length) list.appendChild(h('p', {class: 'vbe-hint', text: 'Nothing matches.'}));
    }
    function run(i) { var c = shown[i]; if (!c) return; dlg.close(); c[2](); }
    input.addEventListener('input', function () { idx = 0; draw(); });
    input.addEventListener('keydown', function (e) {
      if (e.key === 'ArrowDown') { e.preventDefault(); idx = Math.min(shown.length - 1, idx + 1); draw(); }
      if (e.key === 'ArrowUp') { e.preventDefault(); idx = Math.max(0, idx - 1); draw(); }
      if (e.key === 'Enter') { e.preventDefault(); run(idx); }
    });
    draw();
    dlg = h('dialog', {class: 'vbe-dialog vbe-finder'});
    dlg.appendChild(box);
    dlg.addEventListener('close', function () { setTimeout(function () { dlg.remove(); }, 0); });
    document.body.appendChild(dlg);
    dlg.showModal();
    input.focus();
  }

  // ---------------------------------------------------------------------
  // dialogs, toast
  // ---------------------------------------------------------------------
  var toastTimer;
  function toast(msg, error) {
    var t = $('.vbe-toast') || document.body.appendChild(h('div', {class: 'vbe-toast', role: 'status'}));
    t.textContent = msg;
    t.classList.toggle('error', !!error);
    t.classList.add('show');
    clearTimeout(toastTimer);
    toastTimer = setTimeout(function () { t.classList.remove('show'); }, error ? 5000 : 2600);
  }
  function flash(n) { if (n) n.animate([{boxShadow: '0 0 0 0 rgba(255,26,117,.6)'}, {boxShadow: '0 0 0 8px rgba(255,26,117,0)'}], {duration: 700}); }
  function dialog(title, text, body, okLabel) {
    return new Promise(function (resolve) {
      var d = h('dialog', {class: 'vbe-dialog'});
      d.appendChild(h('h2', {text: title}));
      if (text) d.appendChild(h('p', {text: text}));
      if (body) d.appendChild(body);
      d.appendChild(h('div', {class: 'vbe-dlg-actions'}, [
        h('button', {class: 'vbe-btn text', type: 'button', text: 'Cancel', onclick: function () { d.close(); resolve(false); }}),
        h('button', {class: 'vbe-btn primary', type: 'button', text: okLabel || 'OK', onclick: function () { d.close(); resolve(true); }})
      ]));
      d.addEventListener('close', function () { setTimeout(function () { d.remove(); }, 0); });
      d.addEventListener('cancel', function () { resolve(false); });
      document.body.appendChild(d);
      d.showModal();
      var f = d.querySelector('input,textarea');
      if (f) f.focus();
    });
  }

  function cataloguePicker(field, initial) {
    var selected = initial.slice();
    var labels = new Map();
    var offset = 0, request = 0, timer, closed = false;
    var singular = field.of === 'products' ? 'product' : 'listing';
    var search = h('input', {class: 'vbe-in', type: 'search', maxlength: 100, placeholder: 'Search ' + field.label.toLowerCase(), 'aria-label': 'Search ' + field.label.toLowerCase()});
    var chosen = h('div', {class: 'vbe-catalogue-chosen', 'aria-label': 'Selected items'});
    var status = h('p', {role: 'status'});
    var results = h('div', {class: 'vbe-catalogue-results', role: 'group', 'aria-label': 'Search results'});
    var previous = h('button', {class: 'vbe-btn text small', type: 'button', text: 'Previous', disabled: true});
    var next = h('button', {class: 'vbe-btn text small', type: 'button', text: 'Next', disabled: true});

    function fetchItems(query) {
      var params = new URLSearchParams(Object.assign({kind: field.of}, query));
      return fetch(BOOT.endpoints.catalogue + '?' + params, {credentials: 'same-origin', headers: {Accept: 'application/json'}}).then(function (response) {
        if (!response.ok || response.redirected) throw new Error('The catalogue could not be loaded.');
        return response.json();
      });
    }
    function updateSelection() {
      chosen.replaceChildren(h('span', {class: 'vbe-hint', text: selected.length + ' selected'}));
      selected.forEach(function (id) {
        var label = labels.get(id) || 'Unavailable ' + singular;
        chosen.appendChild(h('button', {class: 'vbe-btn tonal small', type: 'button',
          text: label + ' ×', 'aria-label': 'Remove ' + label,
          onclick: function () {
            selected = selected.filter(function (selectedId) { return selectedId !== id; });
            updateSelection();
            var inputs = Array.from(results.querySelectorAll('input'));
            var input = inputs.find(function (item) { return item.value === id; });
            var focus = input || chosen.querySelector('button') || search;
            focus.focus();
          }}));
      });
      results.querySelectorAll('input').forEach(function (input) {
        input.checked = selected.includes(input.value);
        input.disabled = !input.checked && selected.length >= field.max;
      });
    }
    function load() {
      var current = ++request;
      previous.disabled = next.disabled = true;
      results.replaceChildren();
      status.textContent = 'Loading…';
      fetchItems({search: search.value.trim(), offset: String(offset)}).then(function (data) {
        if (closed || current !== request) return;
        data.items.forEach(function (item) {
          var id = item[0], title = item[1];
          labels.set(id, title);
          var input = h('input', {type: 'checkbox', value: id, onchange: function () {
            if (input.checked && selected.length < field.max) selected.push(id);
            else if (!input.checked) selected = selected.filter(function (value) { return value !== id; });
            updateSelection();
          }});
          results.appendChild(h('label', {}, [input, h('span', {text: title})]));
        });
        status.textContent = data.items.length ? '' : 'No matching ' + field.label.toLowerCase() + '.';
        previous.disabled = offset === 0;
        previous.onclick = function () { offset = Math.max(0, offset - data.pageSize); load(); };
        next.disabled = data.next == null;
        next.onclick = function () { if (data.next != null) { offset = data.next; load(); } };
        updateSelection();
      }).catch(function (error) {
        if (!closed && current === request) status.textContent = error.message;
      });
    }
    search.addEventListener('input', function () {
      offset = 0;
      request++;
      clearTimeout(timer);
      timer = setTimeout(load, 180);
    });
    updateSelection();
    if (initial.length) {
      fetchItems({selected: initial.join(',')}).then(function (data) {
        if (closed) return;
        data.items.forEach(function (item) { labels.set(item[0], item[1]); });
        updateSelection();
      }).catch(function () {});
    }
    load();
    var body = h('div', {class: 'vbe-catalogue-picker'}, [search, chosen, status, results,
      h('div', {class: 'vbe-actions-row'}, [previous, next])]);
    return dialog('Choose ' + field.label.toLowerCase(), 'Select up to ' + field.max + ' items. Cards follow your selection order.', body, 'Apply').then(function (ok) {
      closed = true;
      clearTimeout(timer);
      return ok ? selected : null;
    });
  }

  function saveTemplate(kind, data) {
    if (!BOOT.endpoints.templates) return;
    var name = h('input', {class: 'vbe-in', maxlength: '120', placeholder: kind === 'page' ? 'e.g. Product launch page' : 'e.g. Dark call to action'});
    dialog(kind === 'page' ? 'Save the page as a template' : 'Save the section as a template', 'It will be offered under Add → Templates on every page.', name, 'Save template').then(function (ok) {
      if (!ok) return;
      if (!name.value.trim()) { toast('A template needs a name.', true); return; }
      var layout = {v: 2, page: data.page || {}, sections: data.sections};
      if (kind === 'page' && data.elementor) layout.elementor = data.elementor;
      post(BOOT.endpoints.templates, {name: name.value.trim(), kind: kind, data: layout}).then(function (j) {
        S.templates = j.templates || S.templates; S.refs = j.refs || S.refs;
        toast('Template saved.');
        if (S.tab === 'add') renderSide();
      }).catch(function (e) { toast(e.message, true); });
    });
  }
  function exportJson() {
    if (EDITING) finishEditing();
    var title = (IS_TEMPLATE ? T.name : BOOT.post.title) || 'Layout';
    var filename = title.replace(/[^a-z0-9_-]+/gi, '-').replace(/^-|-$/g, '') || 'layout';
    var format = h('select', {class: 'vbe-in', 'aria-label': 'JSON format'}, [
      h('option', {value: 'elementor', text: 'Elementor JSON'}),
      h('option', {value: 'vuuut', text: 'Vuuut JSON'})
    ]);
    var content = h('textarea', {class: 'vbe-ta code', readonly: true, 'aria-label': 'Exported JSON'});
    var status = h('p', {role: 'status'});
    var download = h('button', {class: 'vbe-btn tonal small', type: 'button', text: 'Download JSON', disabled: true, onclick: function () {
      var url = URL.createObjectURL(new Blob([content.value], {type: 'application/json'}));
      var link = h('a', {href: url, download: filename + '.json'});
      document.body.appendChild(link);
      link.click();
      link.remove();
      setTimeout(function () { URL.revokeObjectURL(url); }, 1000);
    }});
    var request = 0;
    function load() {
      var current = ++request;
      download.disabled = true;
      content.value = '';
      status.textContent = 'Preparing JSON…';
      var result = format.value === 'vuuut'
        ? Promise.resolve({document: S.layout})
        : post(BOOT.endpoints.interchange, {export: S.layout, title: title});
      result.then(function (response) {
        if (current !== request) return;
        content.value = JSON.stringify(response.document, null, 2);
        status.textContent = (response.warnings || []).join(' ');
        download.disabled = false;
      }).catch(function (error) {
        if (current === request) status.textContent = error.message;
      });
    }
    format.addEventListener('change', load);
    load();
    dialog('Export layout', 'Use Elementor JSON to import this layout into WordPress.', h('div', {}, [format, content, status, download]), 'Copy JSON').then(function (ok) {
      if (!ok || !content.value) return;
      if (navigator.clipboard) {
        navigator.clipboard.writeText(content.value).then(function () { toast('Copied.'); }, function () { toast('Clipboard access was denied. You can download the JSON instead.', true); });
      } else {
        toast('Clipboard access is unavailable. You can download the JSON instead.', true);
      }
    });
  }

  function importJson() {
    if (EDITING) finishEditing();
    var content = h('textarea', {class: 'vbe-ta code', placeholder: 'Paste JSON', 'aria-label': 'Layout JSON'});
    var file = h('input', {type: 'file', accept: '.json,application/json', 'aria-label': 'Choose a layout JSON file'});
    var status = h('p', {role: 'status'});
    var pending;
    file.addEventListener('change', function () {
      var selected = file.files[0];
      if (!selected) return;
      status.textContent = '';
      pending = selected.size <= BOOT.layoutLimits.bytes
        ? selected.text()
        : Promise.reject(new Error('This layout file is too large.'));
      pending.then(function (text) { content.value = text; }, function (error) { status.textContent = error.message; });
    });
    content.addEventListener('input', function () { pending = null; status.textContent = ''; });
    dialog('Import layout', 'Choose or paste Elementor or Vuuut JSON. Sections go after your selection.', h('div', {}, [file, content, status]), 'Import').then(function (ok) {
      if (!ok) return;
      return (pending || Promise.resolve(content.value)).then(function (text) {
        var document;
        try { document = JSON.parse(text.replace(/^\uFEFF/, '')); }
        catch (_) { throw new Error('The file does not contain valid JSON.'); }
        return post(BOOT.endpoints.interchange, {import: document});
      }).then(function (result) {
        var list = result.layout.sections;
        if (!list.length) throw new Error('The file has no sections.');
        var first = sections()[0];
        var replaceBlank = result.elementor && sections().length === 1 && first.cols.length === 1 &&
          !first.cols[0].items.length && !first.elementor && !Object.keys(first.s).length &&
          !Object.keys(first.cols[0].s).length;
        var total = list.length + (replaceBlank ? 0 : sections().length);
        if (total > BOOT.layoutLimits.sections) throw new Error('The imported sections would exceed the page’s section limit.');
        if (new Blob([JSON.stringify(S.layout), JSON.stringify(result.layout)]).size > BOOT.layoutLimits.bytes - 1024) {
          throw new Error('The imported sections would exceed the layout size limit.');
        }
        insertSections(list, null, {layout: result.layout, replaceBlank: replaceBlank});
        var warnings = result.warnings || [];
        toast(warnings.length ? warnings.join(' ') : 'Layout imported. Save to keep your changes.', !!warnings.length);
      });
    }).catch(function (error) { toast(error.message, true); });
  }

  // ---------------------------------------------------------------------
  // saving
  // ---------------------------------------------------------------------
  function post(url, body) {
    return fetch(url, {method: 'POST', credentials: 'same-origin', headers: {'Content-Type': 'application/json', Accept: 'application/json'}, body: JSON.stringify(body)}).then(function (r) {
      return r.json().catch(function () { return {}; }).then(function (j) {
        if (!r.ok) throw new Error(j.error || (r.status === 401 ? 'You have been signed out — sign in again in another tab, then save.' : 'Saving failed (' + r.status + ').'));
        return j;
      });
    });
  }
  function save() {
    if (S.saving) return Promise.resolve();
    if (EDITING) finishEditing();
    S.saving = true;
    dirty(S.dirty);
    var b = $('#vbe-save'); if (b) b.disabled = true;
    var body = {layout: S.layout, active: S.active};
    if (IS_TEMPLATE) { body.name = S.name; body.options = S.options; }
    else if (BOOT.post.type === 'page') body.showTitle = S.showTitle;
    return post(BOOT.endpoints.save, body).then(function (j) {
      S.saving = false;
      BOOT.savedAt = j.savedAt;
      if (j.layout) { var keep = S.sel; S.layout = j.layout; S.sel = keep && locate(keep) ? keep : null; }
      BOOT.post.showTitle = S.showTitle;
      dirty(false);
      renderAll();
      toast(IS_TEMPLATE ? (T.theme ? (S.active ? 'Saved. It is live wherever its conditions match.' : 'Saved. It is switched off.') : 'Saved. Every page using it has been updated.') :
        S.active ? 'Saved. The layout is live on this ' + BOOT.post.type + (BOOT.post.status === 'published' ? '.' : ' (still a draft).') : 'Saved. The layout is off, so visitors see the original content.');
    }).catch(function (e) {
      S.saving = false;
      dirty(true);
      renderTop();
      toast(e.message, true);
      throw e;
    });
  }
  function preview() {
    var url = BOOT.post.status === 'published' && BOOT.post.viewUrl ? BOOT.post.viewUrl : BOOT.post.previewUrl;
    if (!url) { toast('Save the page in the editor first to get a preview link.', true); return; }
    var w = window.open('about:blank', '_blank');
    (S.dirty ? save() : Promise.resolve()).then(function () { if (w) w.location = url; }).catch(function () { if (w) w.close(); });
  }

  // ---------------------------------------------------------------------
  // keyboard
  // ---------------------------------------------------------------------
  function onKey(e) {
    var mod = e.metaKey || e.ctrlKey;
    var k = e.key.toLowerCase();
    var inField = e.target.closest && e.target.closest('input, textarea, select, [contenteditable=true]');
    if (mod && k === 's') { e.preventDefault(); save(); return; }
    if (mod && (k === 'k' || k === 'e')) { e.preventDefault(); finder(); return; }
    if (inField) return;
    if (e.key === 'Escape' && menuEl) { hideMenu(); return; }
    if (mod && k === 'z') { e.preventDefault(); if (e.shiftKey) redo(); else undo(); return; }
    if (mod && k === 'y') { e.preventDefault(); redo(); return; }
    if (mod && k === 'v') { e.preventDefault(); if (e.shiftKey) pasteStyle(); else paste(); return; }
    if (!S.sel) return;
    if (mod && k === 'c') { e.preventDefault(); copyNode(S.sel); return; }
    if (mod && k === 'd') { e.preventDefault(); duplicate(S.sel); return; }
    if (e.key === 'Delete' || e.key === 'Backspace') { e.preventDefault(); remove(S.sel); return; }
    if (e.altKey && e.key === 'ArrowUp') { e.preventDefault(); shift(S.sel, -1); return; }
    if (e.altKey && e.key === 'ArrowDown') { e.preventDefault(); shift(S.sel, 1); return; }
    if (e.key === 'Escape') {
      var loc2 = locate(S.sel);
      if (!loc2) return;
      select(loc2.kind === 'widget' ? loc2.col.id : loc2.kind === 'column' ? (loc2.owner && loc2.owner.t === 'inner' ? loc2.owner.id : loc2.sec.id) : null);
    }
  }

  function renderAll() {
    renderTop();
    renderSide();
    return renderCanvas();
  }

  // ---------------------------------------------------------------------
  // ready-made sections
  // ---------------------------------------------------------------------
  function W(t, s) { return {id: uid(), t: t, s: s}; }
  function C(items, s) { return {id: uid(), s: s || {}, items: items}; }
  function Sx(cols, s) { return {id: uid(), s: s || {}, cols: cols}; }
  var TEMPLATES = [
    {name: 'Hero', thumb: 'hero', find: 'header banner landing top', make: function () {
      return [Sx([C([
        W('heading', {text: 'NEW · Launching this spring', tag: 'h6', align: 'center', colour: '#ffffff'}),
        W('animated-heading', {before: 'Publishing made', words: 'simple\nbeautiful\nprofitable', style: 'rotate', tag: 'h1', align: 'center', size: 64, colour: '#ffffff', word_colour: '#ffffff'}),
        W('text', {html: '<p>One sentence that tells your reader exactly what they get and why it matters to them.</p>', align: 'center', size: 20, colour: '#ffffff'}),
        W('button', {text: 'Get started', href: '#/portal/signup', style: 'filled', size: 'l', align: 'center', colour: '#ffffff', text_colour: 'c1', text2: 'Learn more', href2: '#features'})
      ])], {bg: 'gradient', bg_colour: 'c1', bg_colour2: 'c2', bg_angle: 135, min_h: 70, pt: 120, pb: 120, width: 820, anim: 'fade', shape_bottom: 'wave', shape_bottom_colour: '#ffffff', shape_bottom_h: 90})];
    }},
    {name: 'Hero with image', thumb: 'split', find: 'header split image landing', make: function () {
      return [Sx([C([
        W('heading', {text: 'Work less. Publish more.', tag: 'h1'}),
        W('text', {html: '<p>Explain the promise in two lines. Keep it about the reader, not about you.</p>', size: 19}),
        W('list', {icon: 'check', items: [{text: 'Set up in five minutes'}, {text: 'No credit card needed'}, {text: 'Cancel any time'}]}),
        W('button', {text: 'Start free trial', href: '#/portal/signup', size: 'l', text2: 'See pricing', href2: '#pricing'})
      ]), C([W('image', {alt: '', radius: 20, img_shadow: true, anim: 'slide-left'})])], {valign: 'center', gap: 56, pt: 96, pb: 96})];
    }},
    {name: 'Slider', thumb: 'hero', find: 'slides slider carousel hero banner', make: function () {
      return [Sx([C([W('slides', clone(WSPEC.slides.defaults))])], {layout: 'full', pt: 0, pb: 0})];
    }},
    {name: 'Features', thumb: 'features', find: 'benefits icons grid services three', make: function () {
      var f = function (icon, title) { return C([W('feature', {icon: icon, title: title, text: 'A short description of this benefit, in words your reader would use.', anim: 'rise', hover_anim: 'float'})]); };
      return [Sx([C([W('heading', {text: 'Why people choose us', tag: 'h2', align: 'center'}), W('text', {html: '<p>Three reasons, each one worth a sentence.</p>', align: 'center', colour: 'c4'})])], {pb: 0, width: 720, anchor: 'features'}),
        Sx([f('bolt', 'Fast by default'), f('shield', 'Private and secure'), f('heart', 'Loved by readers')], {gap: 40, pt: 40})];
    }},
    {name: 'Tabs', thumb: 'faq', find: 'tabs features details', make: function () {
      return [Sx([C([W('heading', {text: 'Everything in one place', tag: 'h2', align: 'center'}), W('tabs', clone(WSPEC.tabs.defaults))])], {width: 900})];
    }},
    {name: 'Call to action', thumb: 'cta', find: 'cta banner signup button band', make: function () {
      return [Sx([C([W('heading', {text: 'Ready to get started?', tag: 'h2'}), W('text', {html: '<p>Join thousands of readers who get it first.</p>', size: 18})]), C([W('button', {text: 'Subscribe now', href: '#/portal/signup', size: 'l', align: 'right', colour: 'c1', align_m: 'left'})], {valign: 'center'})],
        {bg: 'colour', bg_colour: 'c3', colour: '#ffffff', valign: 'center', pt: 72, pb: 72})];
    }},
    {name: 'Testimonials', thumb: 'testimonials', find: 'reviews quotes social proof customers carousel', make: function () {
      return [Sx([C([W('heading', {text: 'What readers say', tag: 'h2', align: 'center'}), W('testimonials', clone(WSPEC.testimonials.defaults))])], {bg: 'colour', bg_colour: 'c5'})];
    }},
    {name: 'Pricing', thumb: 'pricing', find: 'plans prices tiers table subscription', make: function () {
      var p = function (title, price, feat, items) { return C([W('pricing', {title: title, price: price, period: '/month', note: feat ? 'Most teams start here.' : 'Everything to begin.', items: items.map(function (x) { return {text: x}; }), button: 'Choose ' + title, href: '#/portal/signup', featured: feat, badge: feat ? 'Most popular' : ''})]); };
      return [Sx([C([W('heading', {text: 'Simple, honest pricing', tag: 'h2', align: 'center'}), W('text', {html: '<p>Start free. Upgrade when you are ready.</p>', align: 'center', colour: 'c4'})])], {pb: 16, anchor: 'pricing'}),
        Sx([p('Starter', '$0', false, ['Up to 1,000 readers', 'Newsletter', 'Basic analytics']), p('Pro', '$19', true, ['Unlimited readers', 'Paid memberships', 'Advanced analytics', 'Priority support']), p('Business', '$49', false, ['Everything in Pro', 'Team seats', 'Custom domain email'])], {valign: 'stretch', pt: 32, gap: 24})];
    }},
    {name: 'FAQ', thumb: 'faq', find: 'questions accordion help answers', make: function () {
      var a = clone(WSPEC.accordion.defaults); a.schema = true; a.skin = 'boxed';
      return [Sx([C([W('heading', {text: 'Questions, answered', tag: 'h2', align: 'center', mb: 16}), W('accordion', a)])], {width: 760})];
    }},
    {name: 'Numbers', thumb: 'stats', find: 'stats counters metrics numbers results', make: function () {
      var s = function (n, sfx, l) { return C([W('stat', {number: n, suffix: sfx, label: l, align: 'center', animate: true})]); };
      return [Sx([s('25', 'k+', 'Readers every week'), s('98', '%', 'Would recommend us'), s('4.9', '★', 'Average rating'), s('12', 'yrs', 'Of publishing')], {pt: 64, pb: 64})];
    }},
    {name: 'Latest posts', thumb: 'gallery', find: 'posts blog articles grid loop portfolio', make: function () {
      return [Sx([C([W('heading', {text: 'Latest stories', tag: 'h2'}), W('posts', {source: 'latest', count: 6, columns: 3, layout: 'cards', show_image: true, show_excerpt: true, show_meta: true, show_tags: true, filter: true, pagination: 'more'})])])];
    }},
    {name: 'Team', thumb: 'features', find: 'team people staff about', make: function () {
      var m = function (n, r) { return C([W('image-box', {title: n, text: r, radius: 16, align: 'center', hover_anim: 'float'}), W('social', {items: [{net: 'x', href: 'https://x.com/'}, {net: 'linkedin', href: 'https://linkedin.com/'}], align: 'center', size: 20})]); };
      return [Sx([C([W('heading', {text: 'The people behind it', tag: 'h2', align: 'center'})])], {pb: 16}), Sx([m('Alex Morgan', 'Founder'), m('Sam Lee', 'Editor'), m('Priya Shah', 'Design')], {pt: 24})];
    }},
    {name: 'Gallery', thumb: 'gallery', find: 'photos images portfolio grid lightbox', make: function () {
      return [Sx([C([W('heading', {text: 'Recent work', tag: 'h2', align: 'center'}), W('gallery', {images: [], columns: 3, gap: 12, ratio: '4/3', radius: 12, lightbox: true, hover: 'zoom'})])])];
    }},
    {name: 'Launch countdown', thumb: 'stats', find: 'countdown launch timer event coming soon', make: function () {
      var d = new Date(Date.now() + 14 * 864e5).toISOString().slice(0, 16);
      return [Sx([C([W('heading', {text: 'Something new is coming', tag: 'h2', align: 'center', colour: '#ffffff'}), W('countdown', {until: d, labels: true, style: 'boxes', align: 'center', digit_colour: '#ffffff'}), W('signup', {align: 'center'})])], {bg: 'gradient', bg_colour: 'c3', bg_colour2: 'c2', colour: '#ffffff', width: 760, pt: 96, pb: 96})];
    }},
    {name: 'Contact', thumb: 'contact', find: 'contact address map location email phone', make: function () {
      return [Sx([C([W('heading', {text: 'Get in touch', tag: 'h2'}), W('text', {html: '<p>We answer every message, usually within a day.</p>'}),
        W('feature', {icon: 'mail', title: 'hello@example.com', text: 'For anything at all.', layout: 'left', link: 'mailto:hello@example.com'}),
        W('feature', {icon: 'phone', title: '+1 555 0100', text: 'Weekdays, 9 to 5.', layout: 'left', link: 'tel:+15550100'}),
        W('feature', {icon: 'pin', title: 'Visit us', text: 'Times Square, New York', layout: 'left'})]),
        C([W('map', {address: 'Times Square, New York', zoom: 14, height: 420, radius: 16})])], {gap: 48, valign: 'center'})];
    }},
    {name: 'Newsletter signup', thumb: 'newsletter', find: 'subscribe email newsletter signup members portal', make: function () {
      return [Sx([C([W('icon', {icon: 'mail', size: 32, shape: 'circle', bg: 'c1', colour: '#ffffff', align: 'center'}), W('heading', {text: 'Get the newsletter', tag: 'h2', align: 'center'}), W('text', {html: '<p>One email a week. The best of what we publish, nothing else.</p>', align: 'center'}), W('signup', {align: 'center', note: 'No spam. Unsubscribe any time.'})], {box_bg: 'colour', box_colour: 'c5', padding: '48', radius_box: '24'})], {width: 760})];
    }},
    {name: 'Video', thumb: 'video', find: 'video youtube vimeo watch demo', make: function () {
      return [Sx([C([W('heading', {text: 'See it in action', tag: 'h2', align: 'center'}), W('text', {html: '<p>Two minutes is all it takes.</p>', align: 'center', colour: 'c4'}), W('video', {url: WSPEC.video.defaults.url, ratio: '16/9', radius: 16})])], {width: 960})];
    }}
  ];

  function thumb(kind) {
    var P = '#e9b9c8', D = '#b0245d', L = '#f4e3e8', G = '#d9c4ca';
    var r = function (x, y, w, h2, c, rx) { return '<rect x="' + x + '" y="' + y + '" width="' + w + '" height="' + h2 + '" rx="' + (rx || 2) + '" fill="' + c + '"/>'; };
    var body = '';
    switch (kind) {
      case 'hero': body = '<defs><linearGradient id="g" x1="0" x2="1" y1="0" y2="1"><stop offset="0" stop-color="#ff1a75"/><stop offset="1" stop-color="#7c4dff"/></linearGradient></defs><rect width="120" height="64" fill="url(#g)"/>' + r(35, 16, 50, 6, '#fff') + r(28, 26, 64, 4, 'rgba(255,255,255,.7)') + r(44, 38, 14, 8, '#fff', 4) + r(62, 38, 14, 8, 'rgba(255,255,255,.4)', 4); break;
      case 'split': body = r(0, 0, 120, 64, L) + r(10, 16, 44, 6, D) + r(10, 26, 40, 3, G) + r(10, 31, 36, 3, G) + r(10, 40, 16, 7, D, 4) + r(64, 10, 46, 44, P, 6); break;
      case 'features': body = r(0, 0, 120, 64, L) + r(40, 8, 40, 5, D) + [12, 47, 82].map(function (x) { return r(x, 22, 10, 10, P, 3) + r(x, 36, 26, 3, D) + r(x, 42, 24, 2, G) + r(x, 46, 22, 2, G); }).join(''); break;
      case 'cta': body = r(0, 0, 120, 64, '#2a2224') + r(12, 24, 50, 6, '#fff') + r(12, 34, 36, 3, '#8a7a7e') + r(82, 26, 26, 10, '#ff1a75', 5); break;
      case 'testimonials': body = r(0, 0, 120, 64, L) + [8, 44, 80].map(function (x) { return r(x, 12, 32, 40, '#fff', 4) + r(x + 4, 18, 14, 2, '#f5a623') + r(x + 4, 24, 24, 2, G) + r(x + 4, 28, 22, 2, G) + r(x + 4, 42, 8, 6, P, 3); }).join(''); break;
      case 'pricing': body = r(0, 0, 120, 64, L) + [10, 44, 78].map(function (x, i) { return r(x, i === 1 ? 6 : 10, 32, i === 1 ? 52 : 44, '#fff', 4) + r(x + 5, (i === 1 ? 12 : 16), 12, 5, D) + r(x + 5, (i === 1 ? 22 : 26), 20, 2, G) + r(x + 5, (i === 1 ? 26 : 30), 18, 2, G) + r(x + 5, (i === 1 ? 46 : 44), 22, 6, i === 1 ? '#ff1a75' : P, 3); }).join(''); break;
      case 'faq': body = r(0, 0, 120, 64, L) + r(36, 7, 48, 5, D) + [18, 28, 38, 48].map(function (y) { return r(22, y, 76, 7, '#fff', 2) + r(26, y + 2.5, 34, 2, G); }).join(''); break;
      case 'stats': body = r(0, 0, 120, 64, L) + [8, 36, 64, 92].map(function (x) { return r(x + 2, 22, 18, 10, D) + r(x, 36, 22, 3, G); }).join(''); break;
      case 'gallery': body = r(0, 0, 120, 64, L) + [[10, 10], [44, 10], [78, 10], [10, 37], [44, 37], [78, 37]].map(function (p) { return r(p[0], p[1], 32, 22, P, 3); }).join(''); break;
      case 'contact': body = r(0, 0, 120, 64, L) + r(10, 12, 36, 5, D) + [24, 34, 44].map(function (y) { return r(10, y, 6, 6, P, 2) + r(20, y + 1, 26, 3, G); }).join('') + r(60, 8, 52, 48, '#cfe3d4', 4) + '<circle cx="86" cy="30" r="4" fill="#ff1a75"/>'; break;
      case 'newsletter': body = r(0, 0, 120, 64, '#fff') + r(22, 8, 76, 48, L, 8) + r(54, 14, 12, 8, P, 2) + r(38, 27, 44, 5, D) + r(42, 36, 36, 3, G) + r(46, 44, 28, 7, '#ff1a75', 4); break;
      case 'video': body = r(0, 0, 120, 64, L) + r(40, 5, 40, 4, D) + r(22, 13, 76, 44, '#2a2224', 4) + '<path d="M56 28l10 7-10 7z" fill="#fff"/>'; break;
      case 'page': body = r(0, 0, 120, 64, L) + r(10, 6, 100, 16, P, 3) + r(10, 26, 30, 14, '#fff', 2) + r(45, 26, 30, 14, '#fff', 2) + r(80, 26, 30, 14, '#fff', 2) + r(10, 44, 100, 14, D, 3); break;
      default: body = r(0, 0, 120, 64, L) + r(14, 14, 92, 36, '#fff', 4) + r(24, 24, 40, 5, D) + r(24, 34, 60, 3, G);
    }
    return '<svg viewBox="0 0 120 64" preserveAspectRatio="xMidYMid slice" aria-hidden="true">' + body + '</svg>';
  }

  // ---------------------------------------------------------------------
  // start
  // ---------------------------------------------------------------------
  document.addEventListener('keydown', onKey);
  window.addEventListener('beforeunload', function (e) { if (S.dirty) { e.preventDefault(); e.returnValue = ''; } });
  renderTop();
  renderSide();
  initCanvas();
  if (BOOT.importNotice) toast(BOOT.importNotice, true);
  else if (BOOT.isNew) toast('Your text has been brought in as the first section. Nothing changes on the site until you save.');
})();
