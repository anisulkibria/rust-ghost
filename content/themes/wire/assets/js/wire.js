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

    // Times. The server writes absolute times; here they become relative
    // ("12 min ago", "Yesterday, 21:40") in the reader's language and time
    // zone, and stories under three hours old are marked fresh. Refreshed
    // every minute so an open tab stays honest.
    var lang = document.documentElement.lang || undefined;
    var FRESH = 3 * 60 * 60 * 1000;
    var rtf = window.Intl && Intl.RelativeTimeFormat ? new Intl.RelativeTimeFormat(lang, { numeric: 'auto', style: 'short' }) : null;
    var clock = window.Intl ? new Intl.DateTimeFormat(lang, { hour: 'numeric', minute: '2-digit' }) : null;
    var weekday = window.Intl ? new Intl.DateTimeFormat(lang, { weekday: 'long' }) : null;
    var dayMonth = window.Intl ? new Intl.DateTimeFormat(lang, { day: 'numeric', month: 'short' }) : null;
    var dayMonthYear = window.Intl ? new Intl.DateTimeFormat(lang, { day: 'numeric', month: 'short', year: 'numeric' }) : null;

    function upper(text) { return text.charAt(0).toUpperCase() + text.slice(1); }
    function startOfDay(d) { return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime(); }

    function relative(then, now) {
        var diff = now - then;
        if (diff < 0) diff = 0;
        var minutes = Math.round(diff / 60000);
        var days = Math.round((startOfDay(now) - startOfDay(then)) / 86400000);
        if (minutes < 1) return rtf.format(0, 'second');
        if (minutes < 60) return rtf.format(-minutes, 'minute');
        if (days === 0) return rtf.format(-Math.floor(minutes / 60), 'hour');
        if (days === 1) return upper(rtf.format(-1, 'day')) + ', ' + clock.format(then);
        if (days < 7) return upper(weekday.format(then)) + ', ' + clock.format(then);
        return then.getFullYear() === now.getFullYear() ? dayMonth.format(then) : dayMonthYear.format(then);
    }

    function refreshTimes() {
        if (!rtf || !clock) return;
        var now = new Date();
        document.querySelectorAll('time[data-when]').forEach(function (el) {
            var then = new Date(el.getAttribute('datetime'));
            if (isNaN(then)) return;
            el.textContent = relative(then, now);
            el.title = then.toLocaleString(lang);
            var item = el.closest('.tl-item');
            if (item) item.classList.toggle('is-fresh', now - then < FRESH);
        });
    }
    refreshTimes();
    setInterval(refreshTimes, 60000);

    // The dateline shows today in the reader's own time zone.
    var today = document.querySelector('[data-today]');
    if (today && window.Intl) {
        var d = new Date();
        today.textContent = new Intl.DateTimeFormat(lang, { weekday: 'long', day: 'numeric', month: 'long', year: 'numeric' }).format(d);
        today.setAttribute('datetime', d.toISOString().slice(0, 10));
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
