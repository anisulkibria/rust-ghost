/* The Form card: puts one of the site's forms into a post or page.
 *
 * Koenig's cards are Lexical decorator nodes, and the bundle takes extra node
 * classes through KoenigComposer's `nodes` prop — its card menu is built from
 * whichever registered classes carry a static `kgMenu`. So this card is a
 * node class like the built-in ones, defined here rather than patched into
 * the vendored bundle.
 *
 * The card stores one thing: the id of a form. What it writes into the post
 * is an empty placeholder element carrying that id; the site swaps the
 * placeholder for the real form each time the page is served. Nothing the
 * card exports is markup anyone typed — the id is checked against the shape
 * of an id before it is written, here and again on the server.
 *
 * The bundle does not export Lexical's base classes, so the card's base
 * class is taken from the prototype chain of a card it does export. If that
 * shape ever changes, create() returns null and the editor simply runs
 * without this card.
 */
(function () {
  'use strict';

  var TYPE = 'vuuut-form';
  var ID = /^[0-9a-f]{24}$/;
  var ICON_PATH = 'M19 3H5c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h14c1.1 0 2-.9 2-2V5c0-1.1-.9-2-2-2zm0 16H5V5h14v14zM7 7h4v4H7V7zm1.3 1.3v1.4h1.4V8.3H8.3zM13 8h4v2h-4V8zm-6 5h10v2H7v-2zm0 3h7v1.5H7V16z';

  function validId(value) { return typeof value === 'string' && ID.test(value) ? value : ''; }

  /* KoenigDecoratorNode: the parent of the first class in a built-in card's
   * chain that declares isKoenigCard. */
  function cardBase(Koenig) {
    var nodes = Koenig && Koenig.DEFAULT_NODES;
    if (!Array.isArray(nodes)) { return null; }
    var start = null;
    for (var i = 0; i < nodes.length; i++) {
      var node = nodes[i];
      if (typeof node === 'function' && typeof node.getType === 'function' && node.getType() === 'html') { start = node; break; }
    }
    for (var cls = start; cls && cls.prototype; cls = Object.getPrototypeOf(cls)) {
      if (Object.prototype.hasOwnProperty.call(cls.prototype, 'isKoenigCard')) {
        var parent = Object.getPrototypeOf(cls);
        return parent && parent.prototype && typeof parent.prototype.decorate === 'function' ? parent : null;
      }
    }
    return null;
  }

  function create(Koenig, React, config) {
    var Base = cardBase(Koenig);
    if (!Base || !Koenig.KoenigCardWrapper || !React) { return null; }

    var e = React.createElement;
    var forms = Array.isArray(config && config.forms) ? config.forms : [];
    var base = (config && config.basePath) || '';
    var canManage = !!(config && config.canManageForms);
    var INSERT_FORM = {type: 'INSERT_VUUUT_FORM_COMMAND'};

    function Icon(props) {
      return e('svg', Object.assign({viewBox: '0 0 24 24', fill: 'currentColor', 'aria-hidden': 'true'}, props),
        e('path', {d: ICON_PATH}));
    }

    function findForm(id) {
      for (var i = 0; i < forms.length; i++) { if (forms[i].id === id) { return forms[i]; } }
      return null;
    }

    function manageLink(text, path) {
      return e('a', {href: base + path, target: '_blank', rel: 'noopener'}, text);
    }

    /* A schematic of the form — its labels and the rough shape of each
     * field — so the writer can see which form this is. Labels are React
     * text children, so they are shown as typed, never parsed. */
    function Preview(props) {
      var form = props.form;
      var fields = (form.fields || []).slice(0, 8).map(function (field, index) {
        var star = field.required ? ' *' : '';
        var tall = field.kind === 'textarea';
        var compact = field.kind === 'consent' || field.kind === 'radio' || field.kind === 'checkboxes';
        return e('div', {key: index},
          e('div', {className: 'vfc-label'}, field.label + star),
          compact ? null : e('div', {className: 'vfc-box' + (tall ? ' tall' : '')}));
      });
      var more = (form.fields || []).length - fields.length;
      return e('div', {className: 'vfc-preview', 'aria-hidden': 'true'},
        fields,
        more > 0 ? e('div', {className: 'vfc-label'}, '+ ' + more + ' more') : null,
        e('div', {className: 'vfc-button'}, form.submitLabel || 'Send'));
    }

    function FormCard(props) {
      var selected = findForm(props.formId);

      function choose(event) {
        var value = validId(event.target.value);
        var editor = props.editor;
        if (!editor) { return; }
        editor.update(function () {
          var latest = props.node.getLatest();
          if (latest) { latest.getWritable().__formId = value; }
        });
      }

      var options = [e('option', {key: '', value: ''}, 'Choose a form…')].concat(forms.map(function (form) {
        return e('option', {key: form.id, value: form.id}, form.name + (form.active ? '' : ' (closed)'));
      }));
      // A card naming a form that has since been deleted keeps its place,
      // and says so, rather than silently pointing at nothing.
      if (props.formId && !selected) {
        options.push(e('option', {key: props.formId, value: props.formId}, 'Deleted form'));
      }

      var note;
      if (!forms.length) {
        note = e('p', {className: 'vfc-note'}, 'There are no forms yet. ',
          canManage ? manageLink('Create one in Forms', '/ghost/forms/') : 'An administrator can create one under Modules → Forms.',
          canManage ? ', then reload this page.' : '');
      } else if (props.formId && !selected) {
        note = e('p', {className: 'vfc-note'}, 'This form has been deleted, so nothing will appear here on the site. Choose another, or remove this card.');
      } else if (selected && !selected.active) {
        note = e('p', {className: 'vfc-note'}, 'This form is closed. Visitors will see a notice that it is no longer accepting responses.');
      } else if (selected) {
        note = e('p', {className: 'vfc-note'}, 'Visitors see the live form here. ',
          canManage ? manageLink('Edit this form', '/ghost/forms/' + selected.id + '/') : null);
      } else {
        note = e('p', {className: 'vfc-note'}, 'Pick the form to show here.');
      }

      // allow-clickthrough: the card wrapper otherwise swallows the mouse
      // press that opens the dropdown.
      return e('div', {className: 'vfc not-kg-prose', 'data-kg-allow-clickthrough': 'true'},
        e('div', {className: 'vfc-head'}, e(Icon, null), 'Form',
          selected && !selected.active ? e('span', {className: 'vfc-closed'}, 'closed') : null),
        e('select', {value: props.formId || '', onChange: choose, 'aria-label': 'Form to show'}, options),
        selected ? e(Preview, {form: selected}) : null,
        note);
    }

    var liveEditor = null;

    class FormNode extends Base {
      static getType() { return TYPE; }
      static clone(node) { return new FormNode({formId: node.__formId}, node.__key); }
      static importJSON(json) { return new FormNode({formId: json && json.formId}); }
      static getPropertyDefaults() { return {formId: ''}; }
      static get urlTransformMap() { return {}; }

      // Pasting a copied card, or importing HTML that contains one, brings
      // the card back rather than an empty div.
      static importDOM() {
        return {
          div: function (element) {
            var id = element.classList && element.classList.contains('kg-form-card') && validId(element.getAttribute('data-form-id'));
            if (!id) { return null; }
            return {conversion: function () { return {node: new FormNode({formId: id})}; }, priority: 3};
          }
        };
      }

      constructor(dataset, key) {
        super(key);
        this.__formId = validId(dataset && dataset.formId);
      }

      isKoenigCard() { return true; }
      getIcon() { return Icon; }
      getDataset() { return {formId: this.getLatest().__formId}; }
      exportJSON() { return {type: TYPE, version: 1, formId: this.__formId}; }

      // Exactly the element the server looks for. An unset or malformed id
      // exports nothing at all.
      exportDOM() {
        var id = validId(this.__formId);
        if (!id) { return {element: null}; }
        var element = document.createElement('div');
        element.className = 'kg-card kg-form-card';
        element.setAttribute('data-form-id', id);
        return {element: element};
      }

      createDOM() { return document.createElement('div'); }
      updateDOM() { return false; }
      isInline() { return false; }
      hasDynamicData() { return false; }
      hasEditMode() { return false; }
      isEmpty() { return !this.__formId; }
      getTextContent() { return ''; }
      getIsVisibilityActive() { return false; }

      decorate(editor) {
        return e(Koenig.KoenigCardWrapper, {nodeKey: this.getKey(), IndicatorIcon: Icon},
          e(FormCard, {node: this, formId: this.__formId, editor: editor || liveEditor}));
      }
    }

    FormNode.kgMenu = {
      label: 'Form',
      desc: 'Add one of your forms',
      Icon: Icon,
      insertCommand: INSERT_FORM,
      matches: ['form', 'contact'],
      priority: 19,
      shortcut: '/form'
    };

    /* Koenig inserts every card through its own INSERT_CARD_COMMAND, which
     * places the node and selects it. That command object is not exported,
     * and a production Lexical build gives commands no name, so it is
     * recognised by its listener: the one function in the editor's command
     * table whose *parameter* is {cardNode, …}. Every card's own insert
     * handler mentions cardNode too, but in its body, when it calls this. */
    var TAKES_CARD = /^\s*(?:async\s+)?(?:function\s*[\w$]*\s*)?\(\s*\{\s*cardNode\b/;
    function findInsertCard(editor) {
      var found = null;
      if (!editor._commands || typeof editor._commands.forEach !== 'function') { return null; }
      editor._commands.forEach(function (priorities, command) {
        if (found || command === INSERT_FORM || !priorities || typeof priorities.forEach !== 'function') { return; }
        priorities.forEach(function (listeners) {
          if (found || !listeners || typeof listeners.forEach !== 'function') { return; }
          listeners.forEach(function (listener) {
            var source = '';
            try { source = Function.prototype.toString.call(listener); } catch (err) { source = ''; }
            if (TAKES_CARD.test(source)) { found = command; }
          });
        });
      });
      return found;
    }

    /* The fallback, should that listener ever change shape: put the card
     * where the cursor is, using only methods every Lexical node has. The
     * card is not selected afterwards, which is the one thing lost. */
    function insertAtCursor(editor, node) {
      var state = editor._pendingEditorState || editor.getEditorState();
      var selection = state && state._selection;
      var at = null;
      if (selection && selection.anchor && typeof selection.anchor.getNode === 'function') {
        at = selection.anchor.getNode();
      } else if (selection && typeof selection.getNodes === 'function') {
        at = selection.getNodes()[0] || null;
      }
      var block = at && typeof at.getTopLevelElement === 'function' ? (at.getTopLevelElement() || at) : at;
      if (!block || typeof block.insertAfter !== 'function') { return false; }
      // An empty line is where "/form" was typed; the card takes its place
      // above it rather than leaving a blank line before the card.
      if (block.getType() === 'paragraph' && block.getTextContent() === '') {
        block.insertBefore(node);
      } else {
        block.insertAfter(node);
      }
      return true;
    }

    var insertCard = null;
    function register(editor) {
      if (!editor || typeof editor.registerCommand !== 'function') { return; }
      liveEditor = editor;
      editor.registerCommand(INSERT_FORM, function () {
        insertCard = insertCard || findInsertCard(editor);
        // With exactly one form there is nothing to choose between.
        var only = forms.length === 1 ? forms[0].id : '';
        var node = new FormNode({formId: only});
        if (insertCard) {
          editor.dispatchCommand(insertCard, {cardNode: node, openInEditMode: false});
          return true;
        }
        return insertAtCursor(editor, node);
      }, 1);
    }

    return {nodes: Koenig.DEFAULT_NODES.concat([FormNode]), register: register, FormNode: FormNode};
  }

  window.GhostKoenigForms = {create: create};
})();
