(function initTransistorEmbed() {
        const script = document.currentScript;
        if (!script) {
            return;
        }

        // The card's figure wrapper is not part of the published HTML, so
        // several cards can share a parent. Each script only owns the iframe
        // rendered directly before it.
        const el = script.previousElementSibling;
        if (!el || !el.matches('iframe[data-src]')) {
            return;
        }

        // Set src with background color detection
        const baseSrc = el.getAttribute('data-src');

        // Colours are resolved from computed styles only. Reading canvas
        // pixels back is unreliable: anti-fingerprinting browsers randomise
        // getImageData, which sent a different colour on every page load.
        function toChannels(match, max) {
            const [r, g, b] = match.slice(1, 4).map(c => Math.round(Math.min(Math.max(Number(c) / max, 0), 1) * 255));
            const a = match[4] === undefined ? 1 : Number(match[4]);
            return [r, g, b, a].every(Number.isFinite) ? {r, g, b, a} : null;
        }

        function parseRgb(color) {
            const match = color.match(/^rgba?\(([^,]+), ([^,]+), ([^,]+?)(?:, ([^,]+))?\)$/);
            return match ? toChannels(match, 255) : null;
        }

        function parseSrgb(color) {
            const match = color.match(/^color\(srgb (\S+) (\S+) (\S+?)(?: \/ (\S+))?\)$/);
            return match ? toChannels(match, 1) : null;
        }

        // Computed styles keep modern colour spaces such as oklch() as
        // authored, so let CSS convert them to sRGB via color-mix().
        function convertToSrgb(color) {
            const probe = document.createElement('span');
            probe.hidden = true;
            probe.style.setProperty('background-color', 'color-mix(in srgb, ' + color + ' 100%, transparent)', 'important');
            script.parentElement.appendChild(probe);
            const converted = window.getComputedStyle(probe).backgroundColor;
            probe.remove();
            return parseSrgb(converted);
        }

        function findBackground() {
            for (let node = el.parentElement; node; node = node.parentElement) {
                const bg = window.getComputedStyle(node).backgroundColor;
                if (bg === 'transparent') {
                    // Older engines serialise transparent backgrounds as the keyword
                    continue;
                }
                const color = parseRgb(bg) || parseSrgb(bg) || convertToSrgb(bg);
                if (!color) {
                    // Unknown format: let Transistor fall back to its default
                    return null;
                }
                if (color.a > 0) {
                    return color;
                }
            }
            return null;
        }

        const background = findBackground();
        if (background) {
            const hex = [background.r, background.g, background.b].map(c => c.toString(16).padStart(2, '0')).join('');
            const u = new URL(baseSrc);
            u.searchParams.set('background', hex);
            el.src = u.toString();
        } else {
            el.src = baseSrc;
        }

        // Listen for resize messages from the Transistor iframe
        window.addEventListener('message', (event) => {
            const isMessageFromValidSource = (
                event.source === el.contentWindow &&
                event.origin === 'https://partner.transistor.fm'
            );
            if (!isMessageFromValidSource) {
                return;
            }

            const isDataValid = (
                !!event.data &&
                typeof event.data === 'object' &&
                !Array.isArray(event.data) &&
                'type' in event.data &&
                event.data.type === 'resize' &&
                'height' in event.data &&
                typeof event.data.height === 'number' &&
                Number.isSafeInteger(event.data.height) &&
                event.data.height >= 1
            );
            if (!isDataValid) {
                return;
            }

            el.style.height = event.data.height + 'px';
        });
    })()