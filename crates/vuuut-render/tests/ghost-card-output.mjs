// Refreshes `ghost-card-output.json` against a Ghost checkout, by rendering
// every Lexical document in it through the renderer that Ghost itself uses.
// The documents are the fixture's; only the expected HTML is recaptured.
//
//   node ghost-card-output.mjs /path/to/Ghost/current > ghost-card-output.json
//
// The path is a Ghost install with its dependencies present (the directory
// holding Ghost's own `index.js` and `node_modules`). Run it after a Ghost
// upgrade: if a card's markup changed, the parity test fails and the Rust
// renderer is what needs updating — not the fixture's `supported` list.
import {readFileSync} from 'node:fs';
import {createRequire} from 'node:module';
import {resolve} from 'node:path';

const ghostDir = process.argv[2];
if (!ghostDir) {
    console.error('usage: node ghost-card-output.mjs /path/to/Ghost/current');
    process.exit(1);
}

const require = createRequire(resolve(ghostDir, 'index.js'));
const {LexicalHTMLRenderer} = require('@tryghost/kg-lexical-html-renderer');
const {DEFAULT_NODES} = require('@tryghost/kg-default-nodes');

const readJson = (path) => JSON.parse(readFileSync(path, 'utf8'));
const fixture = readJson(new URL('./ghost-card-output.json', import.meta.url));
const renderer = new LexicalHTMLRenderer({nodes: DEFAULT_NODES});

const cases = {};
for (const [name, testCase] of Object.entries(fixture.cases)) {
    cases[name] = {
        lexical: testCase.lexical,
        html: await renderer.render(testCase.lexical, {target: 'html'}),
    };
}

fixture.meta.ghost_version = readJson(resolve(ghostDir, 'package.json')).version;
fixture.meta.renderer = `@tryghost/kg-lexical-html-renderer@${
    readJson(resolve(ghostDir, 'node_modules/@tryghost/kg-lexical-html-renderer/package.json')).version
}`;
fixture.cases = cases;

process.stdout.write(`${JSON.stringify(fixture, null, 2)}\n`);
