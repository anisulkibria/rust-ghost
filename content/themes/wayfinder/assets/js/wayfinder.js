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
    var lang = document.documentElement.lang || undefined;
    var places = (body.getAttribute('data-places') || '').split(',').map(function (s) { return s.trim(); }).filter(Boolean);
    var directory = body.getAttribute('data-directory') || '/places/';

    // Once there are listings, category menus offer only the categories
    // listings use; blog tags stay on their own pages.
    var kindsNode = document.querySelector('[data-listing-kinds]');
    var kinds = kindsNode ? kindsNode.textContent.split(/\s+/).filter(Boolean) : [];
    if (kinds.length) {
        document.querySelectorAll('.kind[data-slug]').forEach(function (chip) {
            if (kinds.indexOf(chip.getAttribute('data-slug')) === -1) chip.closest('li').remove();
        });
        document.querySelectorAll('select[name="what"] option').forEach(function (opt) {
            if (opt.value && kinds.indexOf(opt.value) === -1) opt.remove();
        });
    }

    // The sentence search. Each select grows to fit its choice, so the
    // sentence keeps reading as a sentence. One choice opens that tag's page;
    // two open the All places page already filtered.
    document.querySelectorAll('[data-finder]').forEach(function (form) {
        var selects = form.querySelectorAll('select[data-size]');
        var sizer = null;
        if (!(window.CSS && CSS.supports && CSS.supports('field-sizing', 'content'))) {
            sizer = document.createElement('span');
            sizer.className = 'finder-sizer';
            sizer.setAttribute('aria-hidden', 'true');
            form.appendChild(sizer);
        }
        function fit(select) {
            if (!sizer) return;
            sizer.textContent = select.options[select.selectedIndex].text;
            select.style.width = (sizer.getBoundingClientRect().width + 56) + 'px';
        }
        selects.forEach(function (select) {
            fit(select);
            select.addEventListener('change', function () { fit(select); });
        });
        form.addEventListener('submit', function (e) {
            var what = form.querySelector('[name="what"]');
            var where = form.querySelector('[name="where"]');
            var w = what && what.value;
            var p = where && where.value;
            var single = (w && !p) ? what : (p && !w) ? where : null;
            if (single) {
                var url = single.options[single.selectedIndex].getAttribute('data-url');
                if (url) { e.preventDefault(); window.location.href = url; }
            } else if (!w && !p) {
                e.preventDefault();
                window.location.href = form.getAttribute('action');
            }
        });
    });

    // All places page: filter every listing by category, place and price,
    // and keep the address in step so results can be shared.
    var filters = document.querySelector('[data-filters]');
    var results = document.querySelector('[data-results]');
    if (filters && results) {
        var cards = Array.prototype.slice.call(results.querySelectorAll('[data-listing]'));
        var count = document.querySelector('[data-count]');
        var empty = document.querySelector('[data-empty]');
        var what = filters.querySelector('[name="what"]');
        var where = filters.querySelector('[name="where"]');
        var prices = Array.prototype.slice.call(filters.querySelectorAll('[name="price"]'));
        var params = new URLSearchParams(window.location.search);

        function choose(select, value) {
            if (!select || !value) return;
            for (var i = 0; i < select.options.length; i++) {
                if (select.options[i].value === value) { select.selectedIndex = i; return; }
            }
        }
        choose(what, params.get('what'));
        choose(where, params.get('where'));
        (params.get('price') || '').split(',').forEach(function (v) {
            prices.forEach(function (box) { if (box.value === v) box.checked = true; });
        });

        function apply() {
            var w = what ? what.value : '';
            var p = where ? where.value : '';
            var levels = prices.filter(function (b) { return b.checked; }).map(function (b) { return b.value; });
            var shown = 0;
            cards.forEach(function (card) {
                var c = card.classList;
                var ok = (!w || c.contains('tag-' + w)) &&
                    (!p || c.contains('tag-' + p)) &&
                    (!levels.length || levels.some(function (l) { return c.contains('tag-hash-price-' + l); }));
                card.hidden = !ok;
                if (ok) shown++;
            });
            if (count) {
                var text = shown === 0 ? count.getAttribute('data-none') :
                    shown === 1 ? count.getAttribute('data-one') :
                    count.getAttribute('data-many').replace('%', shown);
                count.textContent = text;
            }
            if (empty) empty.hidden = shown !== 0;
            var q = new URLSearchParams();
            if (w) q.set('what', w);
            if (p) q.set('where', p);
            if (levels.length) q.set('price', levels.join(','));
            var qs = q.toString();
            history.replaceState(null, '', window.location.pathname + (qs ? '?' + qs : ''));
        }
        filters.addEventListener('change', apply);
        filters.addEventListener('reset', function () { setTimeout(apply, 0); });
        filters.addEventListener('submit', function (e) { e.preventDefault(); apply(); });
        apply();
    }

    // Topic pages: "in Miami" chips narrow a category to one place on the
    // All places page. On a place's own page they would mean nothing, so they go.
    document.querySelectorAll('[data-narrow]').forEach(function (nav) {
        var current = nav.getAttribute('data-narrow');
        if (places.indexOf(current) !== -1) { nav.remove(); return; }
        nav.querySelectorAll('[data-slug]').forEach(function (chip) {
            chip.href = directory + '?what=' + encodeURIComponent(current) + '&where=' + encodeURIComponent(chip.getAttribute('data-slug'));
        });
    });

    // Listing facts. The first bullet list of "Label: value" lines in a
    // listing becomes the facts panel: address with directions, phone,
    // website, email, opening hours and anything else, as written.
    var facts = document.querySelector('[data-facts]');
    var listingContent = document.querySelector('[data-listing-content]');

    function label(name) { return facts.getAttribute('data-label-' + name) || ''; }

    function buildFacts(card, content) {
        var list = content.querySelector(':scope > ul');
        if (!list) { card.hidden = true; return; }
        var items = Array.prototype.slice.call(list.children);
        var pairs = [];
        items.forEach(function (li) {
            var m = /^\s*([^:]{2,30}):\s*([\s\S]+)$/.exec(li.textContent);
            if (m) pairs.push({ key: m[1].trim(), value: m[2].trim(), link: li.querySelector('a') });
        });
        if (pairs.length < 2 || pairs.length < items.length * 0.6) { card.hidden = true; return; }

        var week = [null, null, null, null, null, null, null];
        var hoursFound = false;
        var rows = [];
        pairs.forEach(function (pair) {
            var key = pair.key.toLowerCase();
            var days = parseDays(key);
            if (days) {
                hoursFound = setHours(week, days, pair.value) || hoursFound;
            } else if (/^(hours|opening hours|open|opening times)$/.test(key)) {
                pair.value.split(/;|\n/).forEach(function (part) {
                    var m = /^\s*([A-Za-z][A-Za-z\s\-–—]*?)\s*[:\s]\s*(.+)$/.exec(part.trim());
                    if (!m) return;
                    var d = parseDays(m[1].toLowerCase());
                    if (d) hoursFound = setHours(week, d, m[2]) || hoursFound;
                });
            } else if (/^(address|location|where)$/.test(key)) {
                rows.push(row('pin', pair.key, text(pair.value), link('https://www.google.com/maps/search/?api=1&query=' + encodeURIComponent(pair.value), label('directions'), true)));
            } else if (/^(phone|tel|telephone|call)$/.test(key)) {
                rows.push(row('phone', pair.key, link('tel:' + pair.value.replace(/[^\d+]/g, ''), pair.value)));
            } else if (/^(website|web|site|url)$/.test(key)) {
                var href = pair.link ? pair.link.href : (/^https?:\/\//.test(pair.value) ? pair.value : 'https://' + pair.value);
                var shown = href.replace(/^https?:\/\/(www\.)?/, '').replace(/\/$/, '');
                rows.push(row('globe', pair.key, link(href, shown, true)));
            } else if (/^(email|e-mail|mail)$/.test(key)) {
                rows.push(row('mail', pair.key, link('mailto:' + pair.value, pair.value)));
            } else {
                rows.push(row(null, pair.key, text(pair.value)));
            }
        });

        card.textContent = '';
        rows.forEach(function (r) { card.appendChild(r); });
        if (hoursFound) {
            card.appendChild(hoursTable(week));
            showStatus(week);
        }
        list.remove();
    }

    var DAY_NAMES = ['mon', 'tue', 'wed', 'thu', 'fri', 'sat', 'sun'];
    function dayIndex(word) {
        word = word.trim().slice(0, 3);
        return DAY_NAMES.indexOf(word);
    }
    function parseDays(key) {
        key = key.replace(/\s+/g, ' ').trim();
        if (/^(daily|every day|everyday|all week)$/.test(key)) return [0, 1, 2, 3, 4, 5, 6];
        if (/^weekdays?$/.test(key)) return [0, 1, 2, 3, 4];
        if (/^weekends?$/.test(key)) return [5, 6];
        var range = /^([a-z]{3,9})\s*(?:-|–|—|to)\s*([a-z]{3,9})$/.exec(key);
        if (range) {
            var a = dayIndex(range[1]), b = dayIndex(range[2]);
            if (a < 0 || b < 0) return null;
            var out = [];
            for (var i = a; ; i = (i + 1) % 7) { out.push(i); if (i === b) break; }
            return out;
        }
        var list = key.split(/\s*(?:,|&|and)\s*/);
        var days = list.map(dayIndex);
        if (days.length && days.every(function (d) { return d >= 0; }) && list.every(function (w) { return /^[a-z]{3,9}$/.test(w); })) return days;
        return null;
    }

    function toMinutes(h, m, mer) {
        h = parseInt(h, 10); m = parseInt(m || '0', 10);
        if (mer) {
            mer = mer.charAt(0).toLowerCase();
            if (mer === 'p' && h < 12) h += 12;
            if (mer === 'a' && h === 12) h = 0;
        }
        return h * 60 + m;
    }

    function parseIntervals(value) {
        var v = value.toLowerCase().trim();
        if (/closed/.test(v)) return 'closed';
        if (/24\s*h|all day|24\/7|around the clock/.test(v)) return 'allday';
        var out = [];
        var re = /(\d{1,2})(?:[:.](\d{2}))?\s*(a\.?m\.?|p\.?m\.?)?\s*(?:-|–|—|to)\s*(\d{1,2})(?:[:.](\d{2}))?\s*(a\.?m\.?|p\.?m\.?)?/g;
        var m;
        while ((m = re.exec(v))) {
            var endMer = m[6], startMer = m[3];
            if (!startMer && endMer) {
                var h1 = parseInt(m[1], 10), h2 = parseInt(m[4], 10);
                startMer = (endMer.charAt(0) === 'p' && h1 <= h2 && h1 !== 12) ? 'pm' : 'am';
            }
            var start = toMinutes(m[1], m[2], startMer);
            var end = toMinutes(m[4], m[5], endMer);
            if (end <= start) end += 1440;
            out.push([start, end]);
        }
        return out.length ? out : null;
    }

    function setHours(week, days, value) {
        var parsed = parseIntervals(value);
        if (!parsed) return false;
        days.forEach(function (d) { week[d] = parsed; });
        return true;
    }

    var timeFormat = window.Intl ? new Intl.DateTimeFormat(lang, { hour: 'numeric', minute: '2-digit' }) : null;
    function clock(minutes) {
        minutes = minutes % 1440;
        var d = new Date(2000, 0, 3, Math.floor(minutes / 60), minutes % 60);
        return timeFormat ? timeFormat.format(d) : (Math.floor(minutes / 60) + ':' + ('0' + minutes % 60).slice(-2));
    }
    function dayName(i, style) {
        // 3 Jan 2000 was a Monday.
        var d = new Date(2000, 0, 3 + i);
        return window.Intl ? new Intl.DateTimeFormat(lang, { weekday: style || 'long' }).format(d) : DAY_NAMES[i];
    }
    function describe(day) {
        if (day === 'closed' || day === null) return label('closedday');
        if (day === 'allday') return label('allday');
        return day.map(function (r) { return clock(r[0]) + '–' + clock(r[1]); }).join(', ');
    }

    // Now, in the site's own time zone: businesses keep local hours.
    function siteNow() {
        var tz = body.getAttribute('data-timezone') || undefined;
        try {
            var parts = new Intl.DateTimeFormat('en-US', { timeZone: tz, weekday: 'short', hour: 'numeric', minute: 'numeric', hourCycle: 'h23' }).formatToParts(new Date());
            var get = function (type) { var p = parts.filter(function (x) { return x.type === type; })[0]; return p ? p.value : ''; };
            return { day: dayIndex(get('weekday').toLowerCase()), minutes: (parseInt(get('hour'), 10) % 24) * 60 + parseInt(get('minute'), 10) };
        } catch (e) {
            var d = new Date();
            return { day: (d.getDay() + 6) % 7, minutes: d.getHours() * 60 + d.getMinutes() };
        }
    }

    function hoursTable(week) {
        var now = siteNow();
        var wrap = document.createElement('div');
        wrap.className = 'facts-row facts-hours';
        var head = document.createElement('p');
        head.className = 'facts-key';
        head.innerHTML = icon('clock');
        head.appendChild(document.createTextNode(label('hours')));
        wrap.appendChild(head);
        var table = document.createElement('table');
        table.className = 'hours';
        week.forEach(function (day, i) {
            var tr = document.createElement('tr');
            if (i === now.day) tr.className = 'is-today';
            var th = document.createElement('th');
            th.scope = 'row';
            th.textContent = upper(dayName(i, 'long'));
            if (i === now.day) th.setAttribute('aria-label', upper(dayName(i, 'long')) + ', ' + label('today'));
            var td = document.createElement('td');
            td.textContent = describe(day);
            tr.appendChild(th);
            tr.appendChild(td);
            table.appendChild(tr);
        });
        wrap.appendChild(table);
        return wrap;
    }

    function showStatus(week) {
        var el = document.querySelector('[data-status]');
        if (!el) return;
        var now = siteNow();
        var t = now.minutes;
        var today = week[now.day];
        var yesterday = week[(now.day + 6) % 7];
        var openUntil = null;
        if (today === 'allday') openUntil = 'allday';
        if (Array.isArray(today)) today.forEach(function (r) { if (t >= r[0] && t < r[1]) openUntil = r[1]; });
        if (Array.isArray(yesterday)) yesterday.forEach(function (r) { if (r[1] > 1440 && t < r[1] - 1440) openUntil = r[1]; });

        if (openUntil === 'allday') {
            el.textContent = label('allday');
            el.className = 'status is-open';
        } else if (openUntil !== null) {
            el.textContent = label('open') + ', ' + label('closes') + ' ' + clock(openUntil);
            el.className = 'status is-open';
        } else {
            var next = null;
            for (var k = 0; k < 7 && next === null; k++) {
                var d = week[(now.day + k) % 7];
                if (d === 'allday') { next = { k: k, at: 0 }; break; }
                if (!Array.isArray(d)) continue;
                for (var j = 0; j < d.length; j++) {
                    if (k > 0 || d[j][0] > t) { next = { k: k, at: d[j][0] }; break; }
                }
            }
            var when = '';
            if (next) {
                when = ', ' + label('opens') + ' ' + clock(next.at);
                if (next.k === 1) when += ' ' + label('tomorrow');
                else if (next.k > 1) when += ', ' + upper(dayName((now.day + next.k) % 7, 'long'));
            }
            el.textContent = label('closed') + when;
            el.className = 'status is-closed';
        }
        el.hidden = false;
    }

    function upper(s) { return s.charAt(0).toUpperCase() + s.slice(1); }
    function text(value) { var p = document.createElement('span'); p.textContent = value; return p; }
    function link(href, words, external) {
        var a = document.createElement('a');
        a.href = href;
        a.textContent = words;
        if (external) { a.target = '_blank'; a.rel = 'noopener'; }
        return a;
    }
    var ICONS = {};
    document.querySelectorAll('template[data-icon]').forEach(function (t) { ICONS[t.getAttribute('data-icon')] = t.innerHTML; });
    function icon(name) { return ICONS[name] || ''; }
    function row(iconName, key, value, extra) {
        var div = document.createElement('div');
        div.className = 'facts-row';
        var k = document.createElement('p');
        k.className = 'facts-key';
        if (iconName) k.innerHTML = icon(iconName);
        k.appendChild(document.createTextNode(key));
        var v = document.createElement('p');
        v.className = 'facts-value';
        v.appendChild(value);
        div.appendChild(k);
        div.appendChild(v);
        if (extra) {
            var x = document.createElement('p');
            x.className = 'facts-extra';
            x.appendChild(extra);
            div.appendChild(x);
        }
        return div;
    }

    // Built last: the helpers above must be defined first.
    if (facts && listingContent) buildFacts(facts, listingContent);

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
