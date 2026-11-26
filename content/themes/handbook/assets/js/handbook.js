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

    // Derive every section and article number from the same ordered tree.
    var order = (body.getAttribute('data-sections') || '').split(',').map(function (s) { return s.trim(); }).filter(Boolean);
    function sortSections(list) {
        var items = Array.from(list.children);
        if (!order.length) return items;

        var bySlug = new Map();
        items.forEach(function (item) {
            var slug = item.getAttribute('data-section');
            if (order.includes(slug)) bySlug.set(slug, item);
            else item.remove();
        });
        var sections = Array.from(new Set(order)).map(function (slug) { return bySlug.get(slug); }).filter(Boolean);
        sections.forEach(function (section) { list.appendChild(section); });
        return sections;
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
        body.classList.add('is-contents-ready');
        contentsToggle.addEventListener('click', function () {
            var open = contentsToggle.getAttribute('aria-expanded') !== 'true';
            contentsToggle.setAttribute('aria-expanded', String(open));
            body.classList.toggle('is-contents-open', open);
        });
    }

    // Build both responsive outlines from the same headings and targets.
    var docContent = document.querySelector('[data-doc-content]');
    var outlines = document.querySelectorAll('[data-toc]');
    if (docContent) {
        var heads = Array.from(docContent.querySelectorAll(':scope > h2, :scope > h3'));
        var links = heads.map(function (heading, index) {
            if (!heading.id) {
                var base = 'section-' + (index + 1);
                var id = base;
                var suffix = 2;
                while (document.getElementById(id)) id = base + '-' + suffix++;
                heading.id = id;
            }
            var title = heading.textContent.trim();
            var fragment = '#' + encodeURIComponent(heading.id);
            var anchor = document.createElement('a');
            anchor.className = 'anchor';
            anchor.href = fragment;
            anchor.setAttribute('aria-label', docContent.getAttribute('data-label-anchor') || '');
            anchor.textContent = '#';
            heading.appendChild(anchor);

            return Array.from(outlines, function (outline) {
                var item = document.createElement('li');
                if (heading.tagName === 'H3') item.className = 'toc-sub';
                var link = document.createElement('a');
                link.href = fragment;
                link.textContent = title;
                item.appendChild(link);
                outline.querySelector('.toc-list').appendChild(item);
                outline.hidden = false;
                return link;
            });
        });

        if (heads.length && outlines.length) {
            var currentHeading = -1;
            var header = document.querySelector('.site-header');
            var markHeading = function () {
                var readingLine = (header ? header.getBoundingClientRect().bottom : 0) + 36;
                var active = currentHeading;
                while (active >= 0 && heads[active].getBoundingClientRect().top > readingLine) active--;
                while (active + 1 < heads.length && heads[active + 1].getBoundingClientRect().top <= readingLine) active++;
                if (active === currentHeading) return;
                links[currentHeading]?.forEach(function (link) { link.removeAttribute('aria-current'); });
                links[active]?.forEach(function (link) { link.setAttribute('aria-current', 'location'); });
                currentHeading = active;
            };
            var queued = false;
            var queueMark = function () {
                if (queued) return;
                queued = true;
                requestAnimationFrame(function () {
                    queued = false;
                    markHeading();
                });
            };
            window.addEventListener('scroll', queueMark, { passive: true });
            window.addEventListener('resize', queueMark);
            markHeading();
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
