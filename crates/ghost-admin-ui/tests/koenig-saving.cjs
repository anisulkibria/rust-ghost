// Run with: node --test crates/ghost-admin-ui/tests/koenig-saving.cjs
const {test} = require('node:test');
const assert = require('node:assert/strict');
const vm = require('node:vm');
const fs = require('node:fs');
const source = fs.readFileSync(require('node:path').join(__dirname, '../assets/koenig-bridge.js'), 'utf8');

function editor({isNew = true, savedStatus = 'draft', status = 'published', links = []} = {}) {
    const events = () => ({
        listeners: {},
        addEventListener(name, fn) { this.listeners[name] = fn; },
        emit(name, extra = {}) { this.listeners[name]?.({preventDefault() {}, ...extra}); }
    });
    const title = {...events(), value: 'A post worth keeping'};
    const select = {value: status, options: ['draft', 'published', 'scheduled'].map(value => ({value}))};
    const label = {};
    const state = {};
    const side = events();
    const timers = new Map();
    let timerId = 0;
    const requests = [];
    let cardConfig;
    const form = {...events(), action: isNew ? '/ghost/posts/new/' : '/ghost/posts/p1/',
        querySelector(selector) { return selector === 'select[name="status"]' ? select : null; },
        getAttribute() { return this.action; },
        setAttribute(key, value) { this[key] = value; }
    };
    const elements = {
        'koenig-mount': {dataset: {config: JSON.stringify({isNew, savedStatus, postType: 'post'})}},
        'editor-form': form, 'field-title': title, 'field-lexical': {value: ''}, 'field-html': {value: ''},
        'save-state': state, 'editor-view': {hidden: true},
        // The update-collision stamp the editor loaded and posts back.
        'field-updated-at': {value: '2026-10-30 09:00:00'}
    };
    const document = {...events(),
        body: {appendChild(element) { elements[element.id] = element; }},
        createElement() {
            const classes = new Set();
            return {setAttribute(key, value) { this[key] = value; },
                classList: {add(name) { classes.add(name); }, remove(name) { classes.delete(name); },
                    contains(name) { return classes.has(name); }}
            };
        },
        getElementById(id) { return elements[id] || null; },
        querySelector(selector) {
            return selector === '.editor-side' ? side : selector === '[data-save-label]' ? label : null;
        }
    };
    const context = {
        document, console, URLSearchParams,
        window: {...events(), '@tryghost/koenig-lexical': {}, React: {
            createElement(type, props) {
                if (props?.cardConfig) cardConfig = props.cardConfig;
                return {type, props};
            },
            useState(value) { return [value, () => {}]; },
            useRef(value) { return {current: value}; },
            useEffect() {}
        }, ReactDOM: {createRoot() { return {render(element) { element.type(); }}; }}},
        history: {replaceState() {}},
        setTimeout(fn, delay) { timers.set(++timerId, {fn, delay}); return timerId; },
        clearTimeout(id) { timers.delete(id); },
        FormData: class {
            // Stands in for the real form's fields. `updated_at` is a hidden
            // input in the editor, so a browser's FormData carries it and the
            // collision check depends on that.
            forEach(fn) {
                fn(title.value, 'title');
                fn(select.value, 'status');
                fn(elements['field-updated-at'].value, 'updated_at');
            }
        },
        fetch(url, options) {
            if (url.includes('/editor/links/')) return Promise.resolve({ok: true, json: async () => ({links})});
            return new Promise(resolve => requests.push({url, fields: options.body,
                finish(overrides = {}) {
                    const status = options.body.get('status');
                    resolve({ok: true, status: 200, text: async () => JSON.stringify({id: 'p1', status,
                        url: status === 'published' ? 'https://example.com/blog/a-post/' : null,
                        level: 'ok', ...overrides})});
                },
                // A save the server refused, for the update-collision path.
                reject(httpStatus, body) {
                    resolve({ok: false, status: httpStatus,
                        text: async () => JSON.stringify(body)});
                }}));
        }
    };
    vm.runInNewContext(fs.readFileSync(require('node:path').join(__dirname, '../assets/koenig-integrations.js'), 'utf8'), context);
    // Expose the exporter only inside this test's VM.
    vm.runInNewContext(source.replace('  function convertToHtml(editor, serialized) {',
        '  window.exportSavedHtml = convertToHtml;\n  function convertToHtml(editor, serialized) {'), context);
    return {title, select, label, state, side, form, requests, elements, cardConfig,
        exportSavedHtml: context.window.exportSavedHtml,
        // How many autosaves are waiting to fire, so a stopped clock is
        // observable rather than inferred.
        pendingAutosaves() { return [...timers.values()].filter(timer => timer.delay === 3000).length; },
        type() { title.emit('input'); },
        autosave() {
            for (const [id, timer] of timers) {
                if (timer.delay === 3000) { timers.delete(id); timer.fn(); }
            }
        },
        publish() { form.emit('submit'); }
    };
}
const flush = () => new Promise(resolve => setImmediate(resolve));

test('card link search returns groups while settings autocomplete stays flat', async () => {
    const links = [{title: 'A post', url: 'https://example.com/a-post/', label: 'A post', value: 'https://example.com/a-post/'}];
    const e = editor({links});
    const groups = await e.cardConfig.searchLinks('post');
    assert.equal(groups.length, 1);
    assert.equal(groups[0].items, links);
    const autocomplete = await e.cardConfig.fetchAutocompleteLinks('post');
    assert.equal(autocomplete[1], links[0]);
    assert.equal((await editor().cardConfig.searchLinks('missing')).length, 0);
});

test('card HTML uses serialized nested editor values without changing the live state', () => {
    const e = editor();
    const saved = JSON.stringify({calloutText: 'Current callout', textValue: 'Current CTA',
        sponsorLabel: 'Current sponsor', caption: 'Current caption',
        productTitle: 'Current product', productDescription: 'Current product description'});
    let active;
    let parses = 0;
    const state = values => ({
        _nodeMap: new Map([['root', {getChildren: () => Object.entries(values).map(([key]) => ({
            decorate() {},
            exportDOM() { return {element: {outerHTML: `<div>${active[key]}</div>`}}; }
        }))}]]),
        read(fn) { active = values; fn(); }
    });
    const liveState = state({calloutText: 'Old callout', textValue: '', sponsorLabel: '', caption: ''});
    const editorApi = {
        getEditorState: () => liveState,
        parseEditorState(serialized) {
            parses++;
            assert.equal(serialized, saved);
            return state(JSON.parse(serialized));
        },
        setEditorState() { assert.fail('Export must not replace the live editor state'); }
    };
    assert.equal(e.exportSavedHtml(editorApi, saved),
        '<div>Current callout</div><div>Current CTA</div><div>Current sponsor</div><div>Current caption</div>' +
        '<div>Current product</div><div>Current product description</div>');
    assert.equal(parses, 1);
    assert.equal(editorApi.getEditorState(), liveState);
});

test('new post autosaves privately and Publish survives the first draft save', async () => {
    const e = editor();
    assert.equal(e.label.textContent, 'Publish');
    e.type(); e.autosave();
    assert.equal(e.requests[0].fields.get('status'), 'draft');
    e.requests[0].finish(); await flush();
    assert.equal(e.form.action, '/ghost/posts/p1/');
    assert.equal(e.select.value, 'published');
    assert.equal(e.label.textContent, 'Publish');
    e.publish();
    assert.equal(e.requests[1].fields.get('status'), 'published');
    e.requests[1].finish(); await flush();
    assert.equal(e.label.textContent, 'Save post');
});

test('Publish clicked during autosave is queued as a publish request', async () => {
    const e = editor();
    e.type(); e.autosave(); e.publish();
    assert.equal(e.requests.length, 1);
    e.requests[0].finish(); await flush();
    assert.equal(e.requests.length, 2);
    assert.equal(e.requests[1].url, '/ghost/posts/p1/');
    assert.equal(e.requests[1].fields.get('status'), 'published');
});

test('settings cannot publish a draft; selecting Draft keeps explicit saves private', () => {
    const e = editor();
    e.side.emit('change', {target: {name: 'feature_image_caption'}});
    assert.equal(e.requests[0].fields.get('status'), 'draft');
    const draft = editor({status: 'draft'});
    draft.publish();
    assert.equal(draft.requests[0].fields.get('status'), 'draft');
});

test('published content waits for an explicit save and retains its status', () => {
    const e = editor({isNew: false, savedStatus: 'published'});
    e.type(); e.autosave();
    assert.equal(e.requests.length, 0);
    e.publish();
    assert.equal(e.requests[0].fields.get('status'), 'published');
});

test('an autosave response does not clear edits made while the request was in flight', async () => {
    const e = editor();
    e.type(); e.autosave(); e.type();
    e.requests[0].finish(); await flush();
    assert.equal(e.state.textContent, '* Unsaved changes');
    e.autosave();
    assert.equal(e.requests.length, 2);
    assert.equal(e.requests[1].fields.get('status'), 'draft');
});


test('publishing reveals View post and announces success without navigation', async () => {
    const e = editor();
    assert.equal(e.elements['editor-view'].hidden, true);
    e.publish(); e.requests[0].finish(); await flush();
    assert.equal(e.elements['editor-view'].hidden, false);
    assert.equal(e.elements['editor-view'].href, 'https://example.com/blog/a-post/');
    const toast = e.elements['publish-toast'];
    assert.equal(toast.textContent, 'Post published successfully.');
    assert.equal(toast.role, 'status');
    assert.equal(toast.classList.contains('show'), true);
});

test('publishing after the first draft autosave reveals View post', async () => {
    const e = editor();
    e.type(); e.autosave(); e.requests[0].finish(); await flush();
    assert.equal(e.elements['editor-view'].hidden, true);
    assert.equal(e.elements['publish-toast'], undefined);
    e.publish(); e.requests[1].finish(); await flush();
    assert.equal(e.elements['editor-view'].hidden, false);
    assert.equal(e.elements['publish-toast'].textContent, 'Post published successfully.');
});

test('drafts and published post updates do not show a publication toast', async () => {
    const draft = editor({status: 'draft'});
    draft.publish(); draft.requests[0].finish(); await flush();
    assert.equal(draft.elements['editor-view'].hidden, true);
    assert.equal(draft.elements['publish-toast'], undefined);
    const published = editor({isNew: false, savedStatus: 'published'});
    published.publish(); published.requests[0].finish(); await flush();
    assert.equal(published.elements['publish-toast'], undefined);
});

test('unpublishing hides View post and published slug edits update the link', async () => {
    const e = editor({isNew: false, savedStatus: 'published'});
    e.publish(); e.requests[0].finish({url: 'https://example.com/updated-slug/'}); await flush();
    assert.equal(e.elements['editor-view'].href, 'https://example.com/updated-slug/');
    e.select.value = 'draft'; e.publish(); e.requests[1].finish(); await flush();
    assert.equal(e.elements['editor-view'].hidden, true);
});

test('a successful save moves the collision stamp forward', async () => {
    // Without this the second save of a session is compared against the copy
    // the editor first loaded, and the server refuses it as somebody else's.
    const e = editor({isNew: false, savedStatus: 'draft', status: 'draft'});
    e.type();
    e.autosave();
    await flush();
    assert.equal(e.requests[0].fields.get('updated_at'), '2026-10-30 09:00:00',
        'the save posts the stamp it loaded');
    e.requests[0].finish({updated_at: '2026-10-30 09:45:00'});
    await flush();
    assert.equal(e.elements['field-updated-at'].value, '2026-10-30 09:45:00',
        'and adopts the one the save produced');

    e.type();
    e.autosave();
    await flush();
    assert.equal(e.requests[1].fields.get('updated_at'), '2026-10-30 09:45:00',
        'so the next save compares against what it just wrote');
});

test('a refused save keeps the stamp and stops autosaving', async () => {
    const e = editor({isNew: false, savedStatus: 'draft', status: 'draft'});
    e.type();
    e.autosave();
    await flush();
    e.requests[0].reject(409, {message: 'Saving failed! Someone else is editing this post.'});
    await flush();

    assert.equal(e.state.textContent, 'Not saved');
    assert.equal(e.elements['field-updated-at'].value, '2026-10-30 09:00:00',
        'a refused save must not adopt a stamp it did not write');

    // Retrying every three seconds would hit the same wall and bury the
    // notice telling the publisher to copy their work out.
    e.type();
    assert.equal(e.pendingAutosaves(), 0, 'the autosave clock is stopped');
    assert.equal(e.state.textContent, '* Unsaved changes',
        'the edits are still there and still unsaved');
});

test('an ordinary failure still keeps autosaving', async () => {
    // Only a collision stops the clock; a dropped connection should retry.
    const e = editor({isNew: false, savedStatus: 'draft', status: 'draft'});
    e.type();
    e.autosave();
    await flush();
    e.requests[0].reject(500, {message: 'The save failed.'});
    await flush();
    e.type();
    assert.equal(e.pendingAutosaves(), 1);
});
