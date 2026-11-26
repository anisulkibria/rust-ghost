(function () {
    'use strict';

    // Mobile menu
    var toggle = document.querySelector('.menu-toggle');
    var masthead = document.querySelector('.masthead');
    var menu = document.getElementById('site-menu');
    if (toggle && masthead && menu) {
        toggle.addEventListener('click', function () {
            var open = masthead.classList.toggle('is-open');
            toggle.setAttribute('aria-expanded', open ? 'true' : 'false');
            var firstLink = menu.querySelector('a[href]');
            if (open && firstLink) firstLink.focus();
        });
        document.addEventListener('keydown', function (e) {
            if (e.key === 'Escape' && masthead.classList.contains('is-open')) {
                masthead.classList.remove('is-open');
                toggle.setAttribute('aria-expanded', 'false');
                toggle.focus();
            }
        });
    }

    // Make embedded videos responsive when they arrive without a card wrapper
    document.querySelectorAll('.content iframe[src*="youtube"], .content iframe[src*="vimeo"]').forEach(function (frame) {
        if (frame.closest('.kg-embed-card')) return;
        var w = frame.getAttribute('width'), h = frame.getAttribute('height');
        if (w && h) frame.style.aspectRatio = w + ' / ' + h;
        frame.style.width = '100%';
        frame.style.height = 'auto';
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
