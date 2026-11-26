/* Lingua Atlas theme controls. */
(function () {
  function ready(fn) { if (document.readyState !== 'loading') { fn(); } else { document.addEventListener('DOMContentLoaded', fn); } }
  ready(function () {
    var toggle = document.querySelector('.menu-toggle'), nav = document.getElementById('site-nav');
    if (toggle && nav) {
      toggle.addEventListener('click', function () {
        var open = nav.classList.toggle('open');
        toggle.setAttribute('aria-expanded', open ? 'true' : 'false');
      });
      nav.addEventListener('keydown', function (event) {
        if (event.key === 'Escape' && nav.classList.contains('open')) {
          nav.classList.remove('open');
          toggle.setAttribute('aria-expanded', 'false');
          toggle.focus();
        }
      });
    }

    var scheme = document.querySelector('[data-scheme-toggle]');
    if (scheme) {
      scheme.addEventListener('click', function () {
        var root = document.documentElement;
        var now = root.getAttribute('data-scheme');
        var dark = now === 'dark' || (now === 'auto' && window.matchMedia('(prefers-color-scheme: dark)').matches);
        var next = dark ? 'light' : 'dark';
        root.setAttribute('data-scheme', next);
        try { localStorage.setItem('lingua-atlas-scheme', next); } catch (e) { /* private mode */ }
      });
    }

    // Arabic examples sit inside English explanations. Isolate each phrase
    // for direction and pronunciation, preserving any markup from the author.
    var arabicPhrase = /\p{Script=Arabic}[\p{Script=Arabic}\p{M}]*(?: +\p{Script=Arabic}[\p{Script=Arabic}\p{M}]*)*/gu;
    document.querySelectorAll('[data-atlas-arabic]').forEach(function (content) {
      var walker = document.createTreeWalker(content, NodeFilter.SHOW_TEXT);
      var textNodes = [];
      while (walker.nextNode()) textNodes.push(walker.currentNode);
      textNodes.forEach(function (node) {
        var marked = node.parentElement.closest('[lang], [dir], code, pre, script, style');
        if (marked && content.contains(marked)) return;
        var text = node.nodeValue;
        var matches = Array.from(text.matchAll(arabicPhrase));
        if (!matches.length) return;
        var result = document.createDocumentFragment();
        var offset = 0;
        matches.forEach(function (match) {
          result.append(text.slice(offset, match.index));
          var phrase = document.createElement('bdi');
          phrase.lang = 'ar';
          phrase.dir = 'rtl';
          phrase.textContent = match[0];
          result.append(phrase);
          offset = match.index + match[0].length;
        });
        result.append(text.slice(offset));
        node.replaceWith(result);
      });
    });

    // Instructor page: courses, students and the average rating, from the cards.
    var stats = document.querySelector('[data-ins-stats]');
    var cards = document.querySelectorAll('[data-ins-courses] .ins-card');
    if (stats && cards.length) {
      var students = 0, weighted = 0, reviews = 0;
      cards.forEach(function (c) {
        students += parseInt(c.dataset.students || '0', 10) || 0;
        var r = parseFloat(c.dataset.rating || ''), n = parseInt(c.dataset.reviews || '0', 10) || 0;
        if (r && n) { weighted += r * n; reviews += n; }
      });
      stats.querySelector('[data-sum=courses]').textContent = cards.length;
      stats.querySelector('[data-sum=students]').textContent = students.toLocaleString();
      stats.querySelector('[data-sum=rating]').textContent = reviews ? (weighted / reviews).toFixed(1) : '–';
      stats.hidden = false;
    }

    var expand = document.querySelector('[data-expand-all]');
    var sections = Array.from(document.querySelectorAll('.syl'));
    if (expand && sections.length > 1) {
      function anyClosed() { return sections.some(function (section) { return !section.open; }); }
      function updateLabel() { expand.textContent = anyClosed() ? 'Expand all' : 'Collapse all'; }
      sections.forEach(function (section) { section.addEventListener('toggle', updateLabel); });
      expand.addEventListener('click', function () {
        var open = anyClosed();
        sections.forEach(function (section) { section.open = open; });
        updateLabel();
      });
      updateLabel();
      expand.hidden = false;
    }
  });
})();
