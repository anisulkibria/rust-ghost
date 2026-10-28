// Run with: node --test crates/ghost-admin-ui/tests/koenig-saving.cjs
const {test} = require('node:test');
const assert = require('node:assert/strict');
const vm = require('node:vm');
const fs = require('node:fs');
const source = fs.readFileSync(require('node:path').join(__dirname, '../assets/koenig-bridge.js'), 'utf8');

function editor({isNew = true, savedStatus = 'draft', status = 'published'} = {}) {
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
    const form = {...events(), action: isNew ? '/ghost/posts/new/' : '/ghost/posts/p1/',
        querySelector(selector) { return selector === 'select[name="status"]' ? select : null; },
        getAttribute() { return this.action; },
        setAttribute(key, value) { this[key] = value; }
    };
    const elements = {
        'koenig-mount': {dataset: {config: JSON.stringify({isNew, savedStatus, postType: 'post'})}},
        'editor-form': form, 'field-title': title, 'field-lexical': {value: ''}, 'field-html': {value: ''},
        'save-state': state, 'editor-view': {hidden: true}
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
        window: {...events(), '@tryghost/koenig-lexical': {}, React: {createElement() {}},
            ReactDOM: {createRoot() { return {render() {}}; }}},
        history: {replaceState() {}},
        setTimeout(fn, delay) { timers.set(++timerId, {fn, delay}); return timerId; },
        clearTimeout(id) { timers.delete(id); },
        FormData: class {
            forEach(fn) { fn(title.value, 'title'); fn(select.value, 'status'); }
        },
        fetch(url, options) {
            return new Promise(resolve => requests.push({url, fields: options.body, finish(overrides = {}) {
                const status = options.body.get('status');
                resolve({ok: true, text: async () => JSON.stringify({id: 'p1', status,
                    url: status === 'published' ? 'https://example.com/blog/a-post/' : null,
                    level: 'ok', ...overrides})});
            }}));
        }
    };
    vm.runInNewContext(source, context);
    return {title, select, label, state, side, form, requests, elements,
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
