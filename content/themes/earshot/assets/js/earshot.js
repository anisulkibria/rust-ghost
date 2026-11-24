/* Earshot: the player, the tapes, and the small things around them.
 *
 * There is one <audio> element for the whole page. Every play button (the
 * stage, each episode in a list, the dock) drives it, and every view
 * re-renders from it. An episode's audio comes from the first audio card in
 * its show notes. Listening progress, the chosen speed and "played" marks
 * are kept per reader in localStorage; the page works the same without it.
 */
(function () {
    'use strict';

    var doc = document;
    var body = doc.body;
    var STORE = 'earshot:';
    var RATES = [0.75, 1, 1.25, 1.5, 1.75, 2];
    var BACK = 15;
    var FORWARD = 30;
    var HEARD_MARGIN = 30; // the last half-minute counts as finished
    var MIN_SCALE = 90 * 60; // a 90-minute episode fills a list tape
    var chaptersOn = body.getAttribute('data-chapters') !== 'off';

    var CHECK = '<svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M5 12.5l4.5 4.5L19 7.5"/></svg>';

    /* Storage --------------------------------------------------------------- */

    function read(key) {
        try {
            var value = window.localStorage.getItem(STORE + key);
            return value ? JSON.parse(value) : null;
        } catch (e) {
            return null;
        }
    }

    function write(key, value) {
        try {
            window.localStorage.setItem(STORE + key, JSON.stringify(value));
        } catch (e) { /* storage blocked: progress just isn't remembered */ }
    }

    /* Strings --------------------------------------------------------------- */

    var stringsEl = doc.querySelector('[data-strings]');
    var strings = stringsEl ? stringsEl.dataset : {};

    function say(key, fallback, vars) {
        var text = strings[key] || '';
        // A translation that lost its placeholders can't be filled in.
        if (!text || (fallback.indexOf('{') !== -1 && text.indexOf('{') === -1)) text = fallback;
        if (vars) {
            Object.keys(vars).forEach(function (name) {
                text = text.split('{' + name + '}').join(vars[name]);
            });
        }
        return text;
    }

    /* Time ------------------------------------------------------------------ */

    function clock(seconds) {
        seconds = Math.max(0, Math.floor(seconds || 0));
        var h = Math.floor(seconds / 3600);
        var m = Math.floor((seconds % 3600) / 60);
        var s = seconds % 60;
        return (h ? h + ':' + String(m).padStart(2, '0') : String(m)) + ':' + String(s).padStart(2, '0');
    }

    function spoken(seconds) {
        var total = Math.max(1, Math.round((seconds || 0) / 60));
        var h = Math.floor(total / 60);
        var m = total % 60;
        return h ? say('hours', '{h} hr {m} min', { h: h, m: m }) : say('minutes', '{n} min', { n: total });
    }

    function parseClock(text) {
        var parts = String(text).split(':').map(Number);
        if (parts.length < 2 || parts.some(isNaN)) return null;
        return parts.reduce(function (sum, part) { return sum * 60 + part; }, 0);
    }

    function finite(n) {
        return typeof n === 'number' && isFinite(n) && n > 0 ? n : 0;
    }

    /* Reading episodes out of show notes ------------------------------------ */

    // The first audio card is the episode.
    function sourceIn(root) {
        if (!root) return null;
        var audioEl = root.querySelector('.kg-audio-card audio[src]');
        if (!audioEl) return null;
        var card = audioEl.closest('.kg-audio-card');
        var stored = card && card.querySelector('.kg-audio-duration');
        var text = stored ? stored.textContent.trim() : '';
        var duration = text.indexOf(':') !== -1 ? parseClock(text) : parseFloat(text);
        return { src: audioEl.getAttribute('src'), duration: finite(duration), card: card };
    }

    // A chapter is a list item or paragraph that opens with a timestamp:
    // "04:12 The first shift", "[1:02:33] Questions", "18:40 - Coffee".
    var STAMP = /^\s*[[(]?((?:\d{1,2}:)?\d{1,2}:\d{2})[\])]?(?:\s*[-–—:|.]\s*|\s+)(\S[^\n]*)/;

    function chaptersIn(root) {
        if (!root) return [];
        var found = [];
        Array.prototype.forEach.call(root.children, function (el) {
            var items = el.tagName === 'UL' || el.tagName === 'OL' ? el.children : el.tagName === 'P' ? [el] : [];
            Array.prototype.forEach.call(items, function (item) {
                var match = STAMP.exec(item.textContent);
                if (!match) return;
                var time = parseClock(match[1]);
                if (time === null) return;
                found.push({ time: time, title: match[2].trim(), stamp: match[1], el: item });
            });
        });
        // Only a run of rising times is a chapter list; anything else is just
        // a paragraph that happens to start with a time.
        var chapters = [];
        found.forEach(function (chapter) {
            if (!chapters.length || chapter.time > chapters[chapters.length - 1].time) chapters.push(chapter);
        });
        return chapters.length >= 2 ? chapters : [];
    }

    /* Episodes ---------------------------------------------------------------- */

    var episodes = {};
    var pendingStart = {};

    function episodeFor(el) {
        var id = el.getAttribute('data-id');
        if (!episodes[id]) {
            episodes[id] = {
                id: id,
                url: el.getAttribute('data-url'),
                title: el.getAttribute('data-title'),
                art: el.getAttribute('data-art') || body.getAttribute('data-art') || '',
                number: '',
                src: '',
                duration: 0,
                chapters: [],
                full: false,
                fetched: false
            };
        }
        return episodes[id];
    }

    function numberNear(el) {
        var holder = el.closest('[data-stage], [data-episode]');
        var num = holder && holder.querySelector('.stage-number, .episode-number');
        if (!num) return '';
        var digits = num.textContent.replace(/\D+/g, ' ').trim().split(' ').pop();
        return digits || '';
    }

    var progress = {};

    function progressOf(ep) {
        if (!progress[ep.id]) progress[ep.id] = read('p:' + ep.id) || { t: 0, d: 0, done: false };
        return progress[ep.id];
    }

    function keepProgress(ep, value) {
        progress[ep.id] = value;
        write('p:' + ep.id, value);
    }

    /* The one audio element ----------------------------------------------- */

    var audio = new Audio();
    audio.preload = 'metadata';
    var current = null;
    var failed = false;
    var dockArmed = false;
    var lastSave = 0;
    var rate = read('rate');
    if (RATES.indexOf(rate) === -1) rate = 1;

    function load(ep) {
        if (current === ep) return;
        if (current) saveProgress();
        current = ep;
        failed = false;
        audio.src = ep.src;
        audio.defaultPlaybackRate = rate;
        audio.playbackRate = rate;
        var saved = progressOf(ep);
        var start = pendingStart[ep.id] != null ? pendingStart[ep.id] : saved.done ? 0 : saved.t;
        delete pendingStart[ep.id];
        // Before metadata arrives this sets where playback will begin.
        if (start > 0) {
            try { audio.currentTime = start; } catch (e) { /* set again on loadedmetadata */ }
            audio.addEventListener('loadedmetadata', function once() {
                audio.removeEventListener('loadedmetadata', once);
                if (current === ep && audio.currentTime < 1) audio.currentTime = start;
            });
        }
        describeForLockScreen(ep);
        if (chaptersOn && !ep.full && !ep.fetched) fetchChapters(ep);
    }

    function play() {
        dockArmed = true;
        if (failed) {
            // Try the file again from where it stopped.
            var at = audio.currentTime;
            failed = false;
            audio.load();
            if (at > 0) audio.currentTime = at;
        }
        var promise = audio.play();
        if (promise && promise.catch) promise.catch(function () { render(); });
        updateDock();
    }

    function toggle(ep) {
        if (current !== ep) load(ep);
        if (audio.paused) play();
        else audio.pause();
    }

    function playAt(ep, seconds) {
        if (current !== ep) load(ep);
        seekTo(seconds);
        if (audio.paused) play();
    }

    function seekTo(seconds) {
        if (!current) return;
        var limit = finite(audio.duration) || current.duration || seconds;
        try { audio.currentTime = Math.max(0, Math.min(seconds, limit)); } catch (e) { /* not seekable yet */ }
        saveProgress();
        render();
    }

    function skip(ep, delta) {
        if (current !== ep) load(ep);
        seekTo((audio.currentTime || 0) + delta);
    }

    function cycleRate() {
        rate = RATES[(RATES.indexOf(rate) + 1) % RATES.length];
        audio.defaultPlaybackRate = rate;
        audio.playbackRate = rate;
        write('rate', rate);
        render();
    }

    function saveProgress() {
        if (!current) return;
        var t = audio.currentTime || 0;
        var d = finite(audio.duration) || current.duration;
        var before = progressOf(current);
        var done = audio.ended || (d > 0 && t >= d - HEARD_MARGIN) || (before.done && t < 5);
        keepProgress(current, { t: done ? 0 : Math.floor(t), d: Math.round(d), done: done });
    }

    audio.addEventListener('timeupdate', function () {
        render();
        if (Date.now() - lastSave > 5000) {
            lastSave = Date.now();
            saveProgress();
        }
    });
    ['play', 'pause', 'playing', 'waiting', 'seeked', 'ratechange'].forEach(function (type) {
        audio.addEventListener(type, render);
    });
    audio.addEventListener('pause', saveProgress);
    audio.addEventListener('ended', function () {
        saveProgress();
        render();
    });
    audio.addEventListener('loadedmetadata', function () {
        var d = finite(audio.duration);
        if (!current || !d) return;
        var changed = Math.abs(current.duration - d) > 1;
        current.duration = d;
        if (changed) {
            views.forEach(function (view) { if (view.ep === current) buildTape(view); });
            scaleTapes();
        }
        render();
    });
    audio.addEventListener('error', function () {
        if (!audio.getAttribute('src')) return;
        failed = true;
        render();
    });
    window.addEventListener('pagehide', saveProgress);

    /* Lock screen and media keys ------------------------------------------- */

    var sessionReady = false;

    function describeForLockScreen(ep) {
        if (!('mediaSession' in navigator)) return;
        var show = body.getAttribute('data-show') || '';
        try {
            navigator.mediaSession.metadata = new window.MediaMetadata({
                title: (ep.number ? ep.number + '. ' : '') + ep.title,
                artist: show,
                album: show,
                artwork: ep.art ? [{ src: ep.art, sizes: '400x400' }] : []
            });
        } catch (e) { /* MediaMetadata missing */ }
        if (sessionReady) return;
        sessionReady = true;
        var actions = {
            play: function () { play(); },
            pause: function () { audio.pause(); },
            seekbackward: function () { if (current) skip(current, -BACK); },
            seekforward: function () { if (current) skip(current, FORWARD); },
            seekto: function (details) { if (details && details.seekTime != null) seekTo(details.seekTime); }
        };
        Object.keys(actions).forEach(function (action) {
            try { navigator.mediaSession.setActionHandler(action, actions[action]); } catch (e) { /* unsupported action */ }
        });
    }

    /* Chapters for an episode played away from its own page ---------------- */

    function fetchChapters(ep) {
        ep.fetched = true;
        if (!ep.url || !window.fetch || !window.DOMParser) return;
        window.fetch(ep.url, { credentials: 'same-origin' })
            .then(function (response) { return response.ok ? response.text() : ''; })
            .then(function (html) {
                if (!html) return;
                var page = new window.DOMParser().parseFromString(html, 'text/html');
                var found = chaptersIn(page.querySelector('[data-content]'));
                if (!found.length) return;
                ep.full = true;
                ep.chapters = found.map(function (c) { return { time: c.time, title: c.title }; });
                views.forEach(function (view) { if (view.ep === ep) buildTape(view); });
                render();
            })
            .catch(function () { /* chapters are a nicety */ });
    }

    /* Views ------------------------------------------------------------------ */

    var views = [];

    function bindPlayer(el) {
        var ep = episodeFor(el);
        var template = el.querySelector('template[data-source]');
        var root = template ? template.content : doc.querySelector('[data-content]');
        var source = sourceIn(root);
        var compact = el.hasAttribute('data-compact');

        if (!ep.number) ep.number = numberNear(el);
        if (source && !ep.src) ep.src = source.src;
        if (source && !ep.duration) ep.duration = source.duration;

        var view = { el: el, ep: ep, compact: compact, own: !template, parts: [] };

        // A list row only carries the first few words of its show notes, so
        // a chapter list read from it could stop mid-title. Only the stage
        // reads chapters here; the full set comes from the episode's own
        // notes, or from its page the first time it plays.
        if (chaptersOn && root && !compact && !ep.full) {
            var found = chaptersIn(root);
            if (found.length) {
                ep.chapters = found.map(function (c) { return { time: c.time, title: c.title }; });
                if (view.own) {
                    view.notes = found;
                    ep.full = true;
                }
            }
        }

        el.classList.remove('is-pending');
        if (!ep.src) {
            el.classList.add('is-missing');
            return;
        }

        // On its own page the stage takes over the episode's audio card.
        if (view.own && source.card) source.card.classList.add('is-on-stage');

        view.play = el.querySelector('[data-play]');
        view.playLabel = el.querySelector('[data-play-label]');
        view.play.disabled = false;
        view.play.addEventListener('click', function () { toggle(ep); });

        if (compact) {
            view.row = el.closest('[data-episode]');
            view.tape = el.querySelector('[data-tape]');
            view.length = el.querySelector('[data-length]');
        } else {
            view.seek = el.querySelector('[data-scrub]');
            view.elapsed = el.querySelector('[data-elapsed]');
            view.total = el.querySelector('[data-duration]');
            view.chapter = el.querySelector('[data-chapter]');
            view.rateLabel = el.querySelector('[data-rate-label]');
            view.share = el.querySelector('[data-share]');
            view.shareLabel = el.querySelector('[data-share-label]');
            view.shareText = view.shareLabel ? view.shareLabel.textContent : '';
            var download = el.querySelector('[data-download]');
            if (download) {
                download.href = ep.src;
                download.hidden = false;
            }
            var head = doc.createElement('span');
            head.className = 'tape-head';
            el.querySelector('[data-tape]').appendChild(head);
            view.head = head;

            view.seek.disabled = false;
            view.seek.addEventListener('input', function () {
                if (current !== ep) load(ep);
                seekTo(Number(view.seek.value));
            });
            stepKeys(view.seek, function (delta) { skip(ep, delta); });
            el.querySelectorAll('[data-skip]').forEach(function (button) {
                button.addEventListener('click', function () { skip(ep, Number(button.getAttribute('data-skip'))); });
            });
            el.querySelector('[data-rate]').addEventListener('click', cycleRate);
            if (view.share) {
                view.share.addEventListener('click', function () {
                    var url = view.share.getAttribute('data-share-url');
                    var t = current === ep ? audio.currentTime : 0;
                    if (t >= 1) url += '#t=' + clock(t);
                    copyText(url, view.shareLabel, say('copied', 'Link copied'), view.share);
                });
            }
            var number = el.closest('[data-stage]') && el.closest('[data-stage]').querySelector('.stage-number');
            if (number && ep.number.length >= 4) number.classList.add('is-long');
            stagesSeen.observe(el);
        }

        buildTape(view);
        views.push(view);
        return view;
    }

    // Arrow keys on a one-second scrubber would crawl; step by five seconds,
    // and by a minute with Page Up and Page Down.
    function stepKeys(input, step) {
        input.addEventListener('keydown', function (event) {
            var delta = { ArrowLeft: -5, ArrowDown: -5, ArrowRight: 5, ArrowUp: 5, PageDown: -60, PageUp: 60 }[event.key];
            if (!delta) return;
            event.preventDefault();
            step(delta);
        });
    }

    // One splice per chapter, each as long as its chapter.
    function buildTape(view) {
        var track = view.el.querySelector('[data-track]');
        if (!track) return;
        var ep = view.ep;
        var d = ep.duration;
        var parts = [];

        if (!view.compact && ep.chapters.length && d) {
            var marks = ep.chapters.filter(function (c) { return c.time < d; });
            if (marks.length && marks[0].time > 1) marks.unshift({ time: 0, title: '' });
            marks.forEach(function (mark, i) {
                var end = i + 1 < marks.length ? marks[i + 1].time : d;
                if (end > mark.time) parts.push({ start: mark.time, end: end, title: mark.title });
            });
        }
        if (!parts.length) parts.push({ start: 0, end: d || 1, title: '' });

        track.textContent = '';
        parts.forEach(function (part) {
            var splice = doc.createElement('span');
            splice.className = 'tape-splice';
            splice.style.flexGrow = String(Math.max(1, Math.round(part.end - part.start)));
            var bar = doc.createElement('span');
            bar.className = 'tape-bar';
            var fill = doc.createElement('span');
            fill.className = 'tape-fill';
            bar.appendChild(fill);
            splice.appendChild(bar);
            if (part.title) {
                var label = doc.createElement('span');
                label.className = 'tape-label';
                label.textContent = part.title;
                label.setAttribute('aria-hidden', 'true');
                splice.appendChild(label);
            }
            track.appendChild(splice);
            part.splice = splice;
            part.fill = fill;
        });
        view.parts = parts;
        view.el.classList.toggle('has-labels', parts.some(function (p) { return p.title; }));
        if (!view.compact) {
            fitLabels(track);
            if (labelWatch && !view.watched) {
                labelWatch.observe(track);
                view.watched = true;
            }
        }
    }

    // A splice too narrow to name drops its label rather than showing "C…";
    // the readout under the tape names whichever chapter is playing.
    var LABEL_ROOM = 72;

    function fitLabels(track) {
        track.querySelectorAll('.tape-label').forEach(function (label) {
            label.classList.toggle('is-tight', label.parentNode.getBoundingClientRect().width < LABEL_ROOM);
        });
    }

    var labelWatch = 'ResizeObserver' in window ? new ResizeObserver(function (entries) {
        entries.forEach(function (entry) { fitLabels(entry.target); });
    }) : null;

    // List tapes share one scale, so their lengths compare.
    function scaleTapes() {
        var longest = MIN_SCALE;
        views.forEach(function (view) {
            if (view.compact) longest = Math.max(longest, view.ep.duration || 0);
        });
        views.forEach(function (view) {
            if (!view.compact || !view.tape) return;
            var d = view.ep.duration;
            view.tape.classList.toggle('has-length', d > 0);
            view.tape.style.setProperty('--len', (d / longest).toFixed(4));
        });
    }

    function fill(view, t, d) {
        view.parts.forEach(function (part) {
            var amount = d ? Math.max(0, Math.min(1, (t - part.start) / (part.end - part.start))) : 0;
            part.fill.style.setProperty('--fill', amount.toFixed(4));
            part.splice.classList.toggle('is-current', t >= part.start && t < part.end && t > 0);
        });
    }

    function chapterAt(ep, t) {
        var found = null;
        ep.chapters.forEach(function (c, i) { if (t >= c.time) found = i; });
        return found;
    }

    function positionOf(view) {
        var ep = view.ep;
        var active = current === ep;
        var saved = progressOf(ep);
        var t;
        if (active) t = audio.currentTime || 0;
        else if (pendingStart[ep.id] != null) t = pendingStart[ep.id];
        else t = saved.done ? 0 : saved.t;
        var d = (active && finite(audio.duration)) || ep.duration || saved.d || 0;
        return { t: t, d: d, active: active, playing: active && !audio.paused, done: saved.done && !active };
    }

    function renderView(view) {
        var ep = view.ep;
        var at = positionOf(view);
        var started = at.t > 0;

        view.el.classList.toggle('is-playing', at.playing);
        view.el.classList.toggle('has-started', started || at.active);
        if (view.playLabel) {
            var shared = !at.active && pendingStart[ep.id] != null;
            view.playLabel.textContent = at.playing ? say('pause', 'Pause')
                : shared ? say('playFrom', 'Play from {time}', { time: clock(at.t) })
                : started && !at.done ? say('resume', 'Resume')
                : say('play', 'Play');
        }
        fill(view, at.t, at.d);

        if (view.compact) {
            if (view.row) {
                view.row.classList.toggle('is-played', at.done);
                view.row.classList.toggle('is-active', at.active);
            }
            if (view.length) {
                var text = at.done ? say('played', 'Played') : started && at.d ? say('left', '{time} left', { time: spoken(at.d - at.t) }) : at.d ? spoken(at.d) : '';
                var state = (at.done ? '✓' : '') + text;
                if (view.lengthState !== state) {
                    view.lengthState = state;
                    view.length.textContent = text;
                    if (at.done) view.length.insertAdjacentHTML('afterbegin', CHECK);
                }
            }
            return;
        }

        view.elapsed.textContent = clock(at.t);
        view.total.textContent = at.d ? clock(at.d) : '–:––';
        view.seek.max = String(Math.max(1, Math.floor(at.d)));
        view.seek.value = String(Math.floor(at.t));
        view.seek.setAttribute('aria-valuetext', say('of', '{elapsed} of {total}', { elapsed: clock(at.t), total: clock(at.d) }));
        if (view.head) view.head.style.setProperty('--pos', at.d ? Math.min(1, at.t / at.d).toFixed(4) : '0');
        if (view.rateLabel) view.rateLabel.textContent = rate + '×';

        var index = chapterAt(ep, at.t);
        var chapterText = at.active && failed ? say('error', 'The audio didn’t load. Press play to try again.') : started && index !== null ? ep.chapters[index].title : '';
        if (view.chapter.textContent !== chapterText) view.chapter.textContent = chapterText;

        if (view.shareLabel && !view.share.classList.contains('is-done')) {
            view.shareLabel.textContent = at.active && at.t >= 1 ? say('copyAt', 'Copy link at {time}', { time: clock(at.t) }) : view.shareText;
        }

        if (view.notes) markChapters(view, index, at);
    }

    function render() {
        views.forEach(renderView);
        renderDock();
    }

    /* The episode page: chapters in the notes and in the rail -------------- */

    function markChapters(view, index, at) {
        var started = at.t > 0 || at.active;
        view.notes.forEach(function (chapter, i) {
            var isCurrent = started && i === index;
            chapter.el.classList.toggle('is-current-chapter', isCurrent);
            if (chapter.railItem) {
                chapter.railItem.classList.toggle('is-current', isCurrent);
                chapter.railItem.classList.toggle('is-heard', started && index !== null && i < index);
            }
        });
    }

    function cueButton(stamp, seconds) {
        var button = doc.createElement('button');
        button.type = 'button';
        button.className = 'cue';
        button.textContent = stamp;
        button.setAttribute('data-seek', String(seconds));
        button.setAttribute('aria-label', say('jump', 'Jump to {time}', { time: stamp }));
        return button;
    }

    function cueChapter(chapter) {
        var walker = doc.createTreeWalker(chapter.el, NodeFilter.SHOW_TEXT);
        var node = walker.nextNode();
        while (node && !node.nodeValue.trim()) node = walker.nextNode();
        if (!node || node.parentNode.closest('a, button, code')) return;
        var at = node.nodeValue.indexOf(chapter.stamp);
        if (at === -1) return;
        var stampNode = node.splitText(at);
        var rest = stampNode.splitText(chapter.stamp.length);
        node.nodeValue = node.nodeValue.replace(/[[(]\s*$/, '');
        rest.nodeValue = rest.nodeValue.replace(/^\s*[\])]/, '').replace(/^\s*[-–—:|.]?\s*/, ' ');
        stampNode.parentNode.replaceChild(cueButton(chapter.stamp, chapter.time), stampNode);
        var list = chapter.el.parentElement;
        if (list && (list.tagName === 'UL' || list.tagName === 'OL')) list.classList.add('has-cues');
    }

    // "[12:04]" or "(1:02:33)" anywhere else in the notes, such as a transcript.
    var INLINE_STAMP = /[[(]((?:\d{1,2}:)?\d{1,2}:\d{2})[\])]/g;
    var HAS_INLINE_STAMP = /[[(](?:\d{1,2}:)?\d{1,2}:\d{2}[\])]/;

    function cueInline(notes) {
        var walker = doc.createTreeWalker(notes, NodeFilter.SHOW_TEXT, {
            acceptNode: function (node) {
                if (node.parentNode.closest('a, button, code, pre, script, style, .kg-audio-card')) return NodeFilter.FILTER_REJECT;
                return HAS_INLINE_STAMP.test(node.nodeValue) ? NodeFilter.FILTER_ACCEPT : NodeFilter.FILTER_SKIP;
            }
        });
        var nodes = [];
        var node;
        while ((node = walker.nextNode())) nodes.push(node);
        nodes.forEach(function (text) {
            var frag = doc.createDocumentFragment();
            var value = text.nodeValue;
            var last = 0;
            value.replace(INLINE_STAMP, function (whole, stamp, offset) {
                var seconds = parseClock(stamp);
                if (seconds === null) return whole;
                frag.appendChild(doc.createTextNode(value.slice(last, offset)));
                frag.appendChild(cueButton(stamp, seconds));
                last = offset + whole.length;
                return whole;
            });
            frag.appendChild(doc.createTextNode(value.slice(last)));
            text.parentNode.replaceChild(frag, text);
        });
    }

    function buildRail(view) {
        var rail = doc.querySelector('[data-rail]');
        var list = rail && rail.querySelector('[data-chapter-list]');
        if (!list || !view.notes) return;
        view.notes.forEach(function (chapter) {
            var item = doc.createElement('li');
            item.className = 'chapter';
            var button = doc.createElement('button');
            button.type = 'button';
            button.className = 'chapter-button';
            button.setAttribute('data-seek', String(chapter.time));
            var time = doc.createElement('span');
            time.className = 'chapter-time';
            time.textContent = clock(chapter.time);
            var title = doc.createElement('span');
            title.textContent = chapter.title;
            button.appendChild(time);
            button.appendChild(title);
            item.appendChild(button);
            list.appendChild(item);
            chapter.railItem = item;
        });
        rail.hidden = false;
        // One list in the notes holds every chapter: on small screens the
        // rail would only repeat it.
        var parents = view.notes.map(function (chapter) { return chapter.el.parentElement; });
        if (parents.every(function (p) { return p === parents[0] && (p.tagName === 'UL' || p.tagName === 'OL'); })) {
            rail.classList.add('mirrors-notes');
        }
    }

    function setUpEpisodePage(view) {
        var notes = doc.querySelector('[data-content]');
        if (!notes) return;
        if (view.notes) {
            view.notes.forEach(cueChapter);
            buildRail(view);
        }
        cueInline(notes);

        doc.addEventListener('click', function (event) {
            var button = event.target.closest('[data-seek]');
            if (!button || button.tagName !== 'BUTTON') return;
            playAt(view.ep, Number(button.getAttribute('data-seek')));
        });

        // A shared link to a moment: /episode/#t=12:04
        var moment = /^#t=((?:\d{1,2}:)?\d{1,2}:\d{2}|\d+)$/.exec(window.location.hash);
        if (moment) {
            var seconds = moment[1].indexOf(':') === -1 ? Number(moment[1]) : parseClock(moment[1]);
            if (seconds !== null) pendingStart[view.ep.id] = seconds;
        }
    }

    /* Transcripts -------------------------------------------------------------- */

    function markTranscripts() {
        doc.querySelectorAll('.content .kg-toggle-card').forEach(function (card) {
            var heading = card.querySelector('.kg-toggle-heading-text');
            if (!heading || !/transcript/i.test(heading.textContent)) return;
            card.classList.add('is-transcript');
            card.querySelectorAll('.kg-toggle-content > p > strong:first-child').forEach(function (speaker) {
                speaker.textContent = speaker.textContent.replace(/\s*:\s*$/, '');
            });
        });
    }

    /* The dock ----------------------------------------------------------------- */

    var dock = doc.querySelector('[data-dock]');
    var dockParts = dock ? {
        play: dock.querySelector('[data-dock-play]'),
        label: dock.querySelector('[data-play-label]'),
        title: dock.querySelector('[data-dock-title]'),
        elapsed: dock.querySelector('[data-dock-elapsed]'),
        total: dock.querySelector('[data-dock-duration]'),
        chapter: dock.querySelector('[data-dock-chapter]'),
        fill: dock.querySelector('[data-dock-fill]'),
        seek: dock.querySelector('[data-dock-seek]'),
        rate: dock.querySelector('[data-rate-label]')
    } : null;
    var dockTitleFor = null;

    // Stage players on screen. The dock only appears when the episode that
    // is playing has no player in view.
    var onScreen = [];
    var stagesSeen = 'IntersectionObserver' in window ? new IntersectionObserver(function (entries) {
        entries.forEach(function (entry) {
            var i = onScreen.indexOf(entry.target);
            if (entry.isIntersecting && i === -1) onScreen.push(entry.target);
            if (!entry.isIntersecting && i !== -1) onScreen.splice(i, 1);
        });
        updateDock();
    }) : { observe: function () {} };

    function currentInView() {
        return views.some(function (view) {
            return !view.compact && view.ep === current && onScreen.indexOf(view.el) !== -1;
        });
    }

    function updateDock() {
        if (!dock) return;
        var show = !!(dockArmed && current && !currentInView());
        if (dock.hidden === !show) return;
        dock.hidden = !show;
        body.classList.toggle('has-dock', show);
        if (show) renderDock();
    }

    function renderDock() {
        if (!dock || dock.hidden || !current) return;
        var t = audio.currentTime || 0;
        var d = finite(audio.duration) || current.duration;
        dock.classList.toggle('is-playing', !audio.paused);
        dockParts.label.textContent = audio.paused ? say('play', 'Play') : say('pause', 'Pause');
        if (dockTitleFor !== current) {
            dockTitleFor = current;
            dockParts.title.textContent = (current.number ? current.number + '. ' : '') + current.title;
            dockParts.title.href = current.url;
        }
        dockParts.elapsed.textContent = clock(t);
        dockParts.total.textContent = d ? clock(d) : '–:––';
        var index = chapterAt(current, t);
        dockParts.chapter.textContent = failed ? say('error', 'The audio didn’t load. Press play to try again.') : index !== null && t > 0 ? current.chapters[index].title : '';
        dockParts.fill.style.setProperty('--fill', d ? Math.min(1, t / d).toFixed(4) : '0');
        dockParts.seek.max = String(Math.max(1, Math.floor(d)));
        dockParts.seek.value = String(Math.floor(t));
        dockParts.seek.setAttribute('aria-valuetext', say('of', '{elapsed} of {total}', { elapsed: clock(t), total: clock(d) }));
        dockParts.rate.textContent = rate + '×';
    }

    if (dock) {
        dockParts.play.addEventListener('click', function () { if (current) toggle(current); });
        dockParts.seek.addEventListener('input', function () { seekTo(Number(dockParts.seek.value)); });
        stepKeys(dockParts.seek, function (delta) { if (current) skip(current, delta); });
        dock.querySelectorAll('[data-dock-skip]').forEach(function (button) {
            button.addEventListener('click', function () { if (current) skip(current, Number(button.getAttribute('data-dock-skip'))); });
        });
        dock.querySelector('[data-dock-rate]').addEventListener('click', cycleRate);
        dock.querySelector('[data-dock-close]').addEventListener('click', function () {
            audio.pause();
            saveProgress();
            dockArmed = false;
            updateDock();
        });
    }

    /* Copying ---------------------------------------------------------------- */

    function copyText(text, labelEl, doneText, owner) {
        function confirm() {
            if (!labelEl) return;
            var before = labelEl.getAttribute('data-original') || labelEl.textContent;
            labelEl.setAttribute('data-original', before);
            labelEl.textContent = doneText;
            if (owner) owner.classList.add('is-done');
            window.setTimeout(function () {
                labelEl.textContent = before;
                labelEl.removeAttribute('data-original');
                if (owner) owner.classList.remove('is-done');
                render();
            }, 2500);
        }
        function fallback() {
            var field = doc.createElement('textarea');
            field.value = text;
            field.setAttribute('readonly', '');
            field.style.position = 'fixed';
            field.style.opacity = '0';
            body.appendChild(field);
            field.select();
            try { if (doc.execCommand('copy')) confirm(); } catch (e) { /* nothing more to try */ }
            body.removeChild(field);
        }
        if (navigator.clipboard && navigator.clipboard.writeText) {
            navigator.clipboard.writeText(text).then(confirm, fallback);
        } else {
            fallback();
        }
    }

    doc.querySelectorAll('[data-copy]').forEach(function (button) {
        button.addEventListener('click', function () {
            copyText(button.getAttribute('data-copy'), button.querySelector('[data-copy-label]'), say('copied', 'Link copied'), button);
        });
    });

    /* Durations the show notes didn't record -------------------------------- */

    // Read from the file's own metadata, a couple at a time, for list
    // episodes that come into view without a stored duration.
    var probing = 0;
    var probeQueue = [];

    function probeNext() {
        if (probing >= 2 || !probeQueue.length) return;
        var ep = probeQueue.shift();
        if (ep.duration) return probeNext();
        probing += 1;
        var probe = new Audio();
        probe.preload = 'metadata';
        function finish() {
            probing -= 1;
            probe.removeAttribute('src');
            probeNext();
        }
        probe.addEventListener('loadedmetadata', function () {
            var d = finite(probe.duration);
            if (d) {
                ep.duration = d;
                scaleTapes();
                render();
            }
            finish();
        }, { once: true });
        probe.addEventListener('error', finish, { once: true });
        probe.src = ep.src;
    }

    var rowsSeen = 'IntersectionObserver' in window ? new IntersectionObserver(function (entries) {
        entries.forEach(function (entry) {
            if (!entry.isIntersecting) return;
            rowsSeen.unobserve(entry.target);
            var ep = episodes[entry.target.getAttribute('data-id')];
            if (ep && ep.src && !ep.duration && probeQueue.indexOf(ep) === -1) probeQueue.push(ep);
        });
        probeNext();
    }, { rootMargin: '200px' }) : null;

    /* Set up ------------------------------------------------------------------ */

    doc.querySelectorAll('[data-player]').forEach(function (el) {
        var view = bindPlayer(el);
        if (view && view.compact && rowsSeen && !view.ep.duration) rowsSeen.observe(el);
        if (view && view.own) setUpEpisodePage(view);
    });
    scaleTapes();
    markTranscripts();
    render();
    if (doc.fonts && doc.fonts.ready) {
        doc.fonts.ready.then(function () {
            doc.querySelectorAll('.player [data-track]').forEach(fitLabels);
        });
    }

    /* Masthead ---------------------------------------------------------------- */

    var masthead = doc.querySelector('.masthead');
    var menuToggle = doc.querySelector('.menu-toggle');
    if (masthead && menuToggle) {
        menuToggle.addEventListener('click', function () {
            var open = masthead.classList.toggle('is-open');
            menuToggle.setAttribute('aria-expanded', open ? 'true' : 'false');
        });
        doc.addEventListener('keydown', function (event) {
            if (event.key === 'Escape' && masthead.classList.contains('is-open')) {
                masthead.classList.remove('is-open');
                menuToggle.setAttribute('aria-expanded', 'false');
                menuToggle.focus();
            }
        });
    }

    // Browsers without the popover attribute open the follow panel by class.
    var panel = doc.querySelector('[data-follow-panel]');
    var followToggle = doc.querySelector('[data-follow-toggle]');
    if (panel && followToggle && !Object.prototype.hasOwnProperty.call(HTMLElement.prototype, 'popover')) {
        var setOpen = function (open) {
            panel.classList.toggle('is-open', open);
            followToggle.setAttribute('aria-expanded', open ? 'true' : 'false');
        };
        followToggle.addEventListener('click', function () { setOpen(!panel.classList.contains('is-open')); });
        panel.querySelector('[data-follow-close]').addEventListener('click', function () { setOpen(false); followToggle.focus(); });
        doc.addEventListener('keydown', function (event) { if (event.key === 'Escape') setOpen(false); });
    }

    /* Embeds ------------------------------------------------------------------ */

    doc.querySelectorAll('.content iframe[src*="youtube"], .content iframe[src*="vimeo"]').forEach(function (frame) {
        if (frame.closest('.kg-embed-card')) return;
        var w = frame.getAttribute('width');
        var h = frame.getAttribute('height');
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
