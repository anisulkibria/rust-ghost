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
