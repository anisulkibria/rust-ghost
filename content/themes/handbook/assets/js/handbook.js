(function () {
    'use strict';

    // Mobile menu: the nav is always in the page; below the breakpoint the
    // button shows and hides it.
    var toggle = document.querySelector('.menu-toggle');
    var nav = document.getElementById('site-nav');
    if (toggle && nav) {
        var close = function () {
            toggle.setAttribute('aria-expanded', 'false');
            document.body.classList.remove('is-menu-open');
        };
        toggle.addEventListener('click', function () {
            var open = toggle.getAttribute('aria-expanded') !== 'true';
            toggle.setAttribute('aria-expanded', String(open));
            document.body.classList.toggle('is-menu-open', open);
        });
        document.addEventListener('keydown', function (e) {
            if (e.key === 'Escape' && document.body.classList.contains('is-menu-open')) {
                close();
                toggle.focus();
            }
        });
        nav.addEventListener('click', function (e) {
            if (e.target.closest('a')) close();
        });
    }

    var body = document.body;
    function path(href) {
        try { return new URL(href, window.location.href).pathname.replace(/\/?$/, '/'); } catch (e) { return href; }
    }

    // "/" opens search, as in most documentation sites.
    document.addEventListener('keydown', function (e) {
        if (e.key !== '/' || e.metaKey || e.ctrlKey || e.altKey) return;
        var t = e.target;
        if (t && (t.isContentEditable || /^(input|textarea|select)$/i.test(t.tagName))) return;
        var trigger = document.querySelector('[data-ghost-search]');
        if (trigger) { e.preventDefault(); trigger.click(); }
    });

    // Section order and manual numbering. The Sections setting gives the
    // order; sections not named there follow alphabetically. Every section
    // gets a number (2) and every article in it a number (2.3), taken from
    // the contents tree, so the same address appears wherever it is shown.
    var order = (body.getAttribute('data-sections') || '').split(',').map(function (s) { return s.trim(); }).filter(Boolean);
    function rank(el) {
        var i = order.indexOf(el.getAttribute('data-section'));
        return i === -1 ? order.length : i;
    }
    function sortSections(list) {
        var items = Array.prototype.slice.call(list.children).filter(function (li) { return li.hasAttribute('data-section'); });
        items.map(function (li, i) { return { li: li, r: rank(li), i: i }; })
            .sort(function (a, b) { return a.r - b.r || a.i - b.i; })
            .forEach(function (x) { list.appendChild(x.li); });
        return Array.prototype.slice.call(list.children).filter(function (li) { return li.hasAttribute('data-section'); });
    }

    var numbers = {};
    document.querySelectorAll('[data-tree] .tree-sections').forEach(function (list) {
        sortSections(list).forEach(function (section, i) {
            var n = String(i + 1);
            var head = section.querySelector('.tree-heading a');
            if (head) { head.querySelector('[data-num]').textContent = n; numbers[path(head.href)] = n; }
            section.querySelectorAll('[data-article]').forEach(function (a, j) {
                var m = n + '.' + (j + 1);
                a.querySelector('[data-num]').textContent = m;
                numbers[path(a.href)] = m;
            });
        });
    });
    document.querySelectorAll('[data-ordered]').forEach(function (list) {
        sortSections(list).forEach(function (section, i) {
            var num = section.querySelector('[data-num]');
            if (num) num.textContent = String(i + 1);
        });
    });
    document.querySelectorAll('[data-num-for]').forEach(function (el) {
        var n = numbers[path(el.getAttribute('data-num-for'))];
        if (n) el.textContent = n;
    });

    // Where am I: mark the current page in the tree, keep its section open
    // and fold the others, and bring it into view.
    var here = path(window.location.href);
    var current = null;
    document.querySelectorAll('.doc-tree [data-tree] a').forEach(function (a) {
        if (path(a.href) === here) { a.setAttribute('aria-current', 'page'); current = a; }
    });
    if (current) {
        var mine = current.closest('details');
        document.querySelectorAll('.doc-tree details').forEach(function (d) { d.open = d === mine; });
        var box = document.querySelector('.doc-tree');
        if (box && box.scrollHeight > box.clientHeight) {
            box.scrollTop = Math.max(0, current.offsetTop - box.clientHeight / 3);
        }
    }

    // Contents button on small screens.
    var contentsToggle = document.querySelector('[data-contents-toggle]');
    if (contentsToggle) {
        contentsToggle.addEventListener('click', function () {
            var open = contentsToggle.getAttribute('aria-expanded') !== 'true';
            contentsToggle.setAttribute('aria-expanded', String(open));
            body.classList.toggle('is-contents-open', open);
        });
    }

    // On this page: the article's h2 and h3, with the one being read marked.
    var docContent = document.querySelector('[data-doc-content]');
    var toc = document.querySelector('[data-toc]');
    if (docContent) {
        var heads = Array.prototype.slice.call(docContent.querySelectorAll(':scope > h2, :scope > h3'));
        heads.forEach(function (h, i) {
            if (!h.id) h.id = 'section-' + (i + 1);
            var a = document.createElement('a');
            a.className = 'anchor';
            a.href = '#' + h.id;
            a.setAttribute('aria-label', docContent.getAttribute('data-label-anchor') || '');
            a.textContent = '#';
            h.appendChild(a);
        });
        if (toc && heads.length >= 2) {
            var list = toc.querySelector('.toc-list');
            var links = {};
            heads.forEach(function (h) {
                var li = document.createElement('li');
                li.className = h.tagName === 'H3' ? 'toc-sub' : '';
                var a = document.createElement('a');
                a.href = '#' + h.id;
                a.textContent = h.firstChild ? h.childNodes[0].textContent.trim() : h.textContent;
                li.appendChild(a);
                list.appendChild(li);
                links[h.id] = a;
            });
            toc.hidden = false;
            if ('IntersectionObserver' in window) {
                var visible = {};
                var mark = function () {
                    var first = heads.filter(function (h) { return visible[h.id]; })[0];
                    if (!first) return;
                    Object.keys(links).forEach(function (id) { links[id].classList.toggle('is-current', id === first.id); });
                };
                var io = new IntersectionObserver(function (entries) {
                    entries.forEach(function (en) { visible[en.target.id] = en.isIntersecting; });
                    mark();
                }, { rootMargin: '0px 0px -65% 0px' });
                heads.forEach(function (h) { io.observe(h); });
            }
        }
    }

    // Was this page helpful? Answers stay in this browser; "No" points to support.
    document.querySelectorAll('[data-feedback]').forEach(function (box) {
        var key = 'handbook-helpful:' + window.location.pathname;
        function show(answer) {
            box.querySelector('.feedback-buttons').hidden = true;
            var t = box.querySelector('[data-thanks="' + answer + '"]');
            if (t) t.hidden = false;
        }
        try { var saved = localStorage.getItem(key); if (saved) show(saved); } catch (e) { /* not remembered */ }
        box.querySelectorAll('[data-answer]').forEach(function (b) {
            b.addEventListener('click', function () {
                var answer = b.getAttribute('data-answer');
                try { localStorage.setItem(key, answer); } catch (e) { /* not remembered */ }
                show(answer);
            });
        });
    });

    // Copy link button at the end of a post.
    document.querySelectorAll('[data-copy-url]').forEach(function (button) {
        button.addEventListener('click', function () {
            var url = button.getAttribute('data-copy-url');
            var done = function () {
                button.classList.add('is-done');
                setTimeout(function () { button.classList.remove('is-done'); }, 2400);
            };
            if (navigator.clipboard && navigator.clipboard.writeText) {
                navigator.clipboard.writeText(url).then(done, function () { window.prompt('', url); });
            } else {
                window.prompt('', url);
            }
        });
    });

    // Embedded videos keep their aspect ratio inside the reading column.
    document.querySelectorAll('.gh-content iframe[src*="youtube"], .gh-content iframe[src*="vimeo"]').forEach(function (frame) {
        if (frame.closest('.kg-embed-card') === null && frame.width && frame.height) {
            frame.style.aspectRatio = frame.width + ' / ' + frame.height;
            frame.style.width = '100%';
            frame.style.height = 'auto';
        }
    });
})();

// Light/dark switch. The site's setting picks where readers start; a
// reader's own choice is kept in localStorage and applied in <head>.
(function () {
    'use strict';
    var root = document.documentElement;
    var buttons = document.querySelectorAll('[data-scheme-toggle]');
    if (!buttons.length) return;
    var device = window.matchMedia ? window.matchMedia('(prefers-color-scheme: dark)') : null;

    function shown() {
        var scheme = root.getAttribute('data-scheme');
        if (scheme === 'light' || scheme === 'dark') return scheme;
        return device && device.matches ? 'dark' : 'light';
    }

    function label() {
        var offer = shown() === 'dark' ? 'data-label-light' : 'data-label-dark';
        buttons.forEach(function (button) {
            button.setAttribute('aria-label', button.getAttribute(offer));
        });
    }

    buttons.forEach(function (button) {
        button.addEventListener('click', function () {
            var next = shown() === 'dark' ? 'light' : 'dark';
            root.setAttribute('data-scheme', next);
            try { window.localStorage.setItem('vuuut-scheme', next); } catch (e) { /* not remembered */ }
            label();
        });
    });
    if (device && device.addEventListener) device.addEventListener('change', label);
    label();
})();
