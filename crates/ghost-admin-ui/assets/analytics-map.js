/* The Locations choropleth.
 *
 * An enhancement over a table that is already on the page: the figures are
 * server-rendered beneath this, so a reader who never loads this file — or
 * whose fetch fails — loses a picture, not the data. That is also why the map
 * is painted into a frame that starts with a "loading" line rather than a
 * reserved blank: nothing below it moves when the shapes arrive.
 *
 * The country paths come from Natural Earth via a versioned, immutable asset,
 * so they are fetched once and cached for a year however many times the
 * Analytics page is opened.
 */
(function () {
  'use strict';

  var SVG = 'http://www.w3.org/2000/svg';
  // One request per page even when several maps are on it, and one per
  // session thereafter: the browser cache handles the rest.
  var pending = null;

  function worldData(src) {
    if (!pending) {
      pending = fetch(src, {credentials: 'same-origin'}).then(function (r) {
        if (!r.ok) throw new Error('The map could not be loaded.');
        return r.json();
      });
    }
    return pending;
  }

  function el(name, attrs) {
    var node = document.createElementNS(SVG, name);
    for (var key in attrs) {
      if (attrs[key] !== null && attrs[key] !== undefined) node.setAttribute(key, attrs[key]);
    }
    return node;
  }

  /* Which colour class a country falls in. The cut points arrive from the
     server in data-edges, which is also what the table's swatches were
     rendered from — one definition, so a row and the shape it names can never
     end up different shades. The fallback only matters if that attribute is
     somehow missing. */
  var FALLBACK_EDGES = [0.08, 0.22, 0.42, 0.68, 1.0];
  function binOf(value, max, edges) {
    if (!(max > 0) || !(value > 0)) return 0;
    var share = value / max;
    for (var i = 0; i < edges.length; i++) {
      if (share <= edges[i]) return i;
    }
    return edges.length - 1;
  }

  function draw(figure) {
    var src = figure.getAttribute('data-src');
    var frame = figure.querySelector('.map-frame');
    var edges = (figure.getAttribute('data-edges') || '')
      .split(',')
      .map(Number)
      .filter(function (n) { return !isNaN(n); });
    if (!edges.length) edges = FALLBACK_EDGES;
    var values;
    try {
      values = JSON.parse(figure.getAttribute('data-values') || '{}');
    } catch (error) {
      values = {};
    }

    var max = 0;
    for (var code in values) { if (values[code] > max) max = values[code]; }

    worldData(src).then(function (world) {
      var svg = el('svg', {
        viewBox: world.viewBox,
        class: 'map-svg',
        role: 'img',
        'aria-label': 'Visitors by country. The same figures are listed in the table below.'
      });
      // preserveAspectRatio default (xMidYMid meet) keeps the projection
      // honest — stretching a map to fill a card is a lie about distance.
      var frag = document.createDocumentFragment();

      world.countries.forEach(function (country) {
        var code = country.id.toUpperCase();
        var value = values[code] || 0;
        var path = el('path', {
          d: country.d,
          class: value > 0 ? 'c b' + binOf(value, max, edges) : 'c none',
          'data-code': code
        });
        // A native tooltip, so the hover layer needs no script of its own and
        // keyboard users reading the table see the same words.
        var title = document.createElementNS(SVG, 'title');
        title.textContent = value > 0
          ? country.name + ': ' + value.toLocaleString() + (value === 1 ? ' visitor' : ' visitors')
          : country.name + ': no visitors';
        path.appendChild(title);
        frag.appendChild(path);
      });

      svg.appendChild(frag);
      frame.replaceChildren(svg);
      figure.classList.add('ready');
    }).catch(function () {
      // The table below carries every figure, so a failed map says so quietly
      // and gets out of the way rather than leaving a spinner forever.
      frame.replaceChildren();
      figure.classList.add('failed');
    });
  }

  function start() {
    var maps = document.querySelectorAll('[data-world-map]');
    if (!maps.length) return;
    maps.forEach(draw);
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', start);
  } else {
    start();
  }
})();
