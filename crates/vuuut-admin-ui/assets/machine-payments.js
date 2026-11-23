(function () {
    'use strict';
    var host = document.getElementById('machine-payments');
    if (!host) return;
    var base = location.pathname.split('/ghost/')[0] + '/ghost/api/admin/machine-payments/';

    async function request(path, method, value) {
        var response = await fetch(base + path, {method: method || 'GET', credentials: 'same-origin', cache: 'no-store', headers: {'Content-Type': 'application/json'}, body: value === undefined ? undefined : JSON.stringify(value)});
        var data = response.status === 204 ? {} : await response.json();
        if (!response.ok) throw new Error(data.errors && data.errors[0] && data.errors[0].message || 'The request failed');
        return data;
    }
    function element(tag, text, parent) {
        var node = document.createElement(tag);
        if (text) node.textContent = text;
        if (parent) parent.appendChild(node);
        return node;
    }

    // Material icon paths, by the names `svg()` uses server-side. Only the
    // handful this page needs — duplicating the whole set here would be a
    // second copy to keep in step.
    var ICONS = {
        tune: 'M3 17v2h6v-2H3zM3 5v2h10V5H3zm10 16v-2h8v-2h-8v-2h-2v6h2zM7 9v2H3v2h4v2h2V9H7zm14 4v-2H11v2h10zm-6-4h2V7h4V5h-4V3h-2v6z',
        lock: 'M18 8h-1V6c0-2.76-2.24-5-5-5S7 3.24 7 6v2H6c-1.1 0-2 .9-2 2v10c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V10c0-1.1-.9-2-2-2zm-6 9c-1.1 0-2-.9-2-2s.9-2 2-2 2 .9 2 2-.9 2-2 2zm3.1-9H8.9V6c0-1.71 1.39-3.1 3.1-3.1s3.1 1.39 3.1 3.1v2z',
        globe: 'M11.99 2C6.47 2 2 6.48 2 12s4.47 10 9.99 10C17.52 22 22 17.52 22 12S17.52 2 11.99 2zm6.93 6h-2.95c-.32-1.25-.78-2.45-1.38-3.56 1.84.63 3.37 1.91 4.33 3.56zM12 4.04c.83 1.2 1.48 2.53 1.91 3.96h-3.82c.43-1.43 1.08-2.76 1.91-3.96zM4.26 14C4.1 13.36 4 12.69 4 12s.1-1.36.26-2h3.38c-.08.66-.14 1.32-.14 2 0 .68.06 1.34.14 2H4.26zm.82 2h2.95c.32 1.25.78 2.45 1.38 3.56-1.84-.63-3.37-1.9-4.33-3.56zm2.95-8H5.08c.96-1.66 2.49-2.93 4.33-3.56C8.81 5.55 8.35 6.75 8.03 8zM12 19.96c-.83-1.2-1.48-2.53-1.91-3.96h3.82c-.43 1.43-1.08 2.76-1.91 3.96zM14.34 14H9.66c-.09-.66-.16-1.32-.16-2 0-.68.07-1.35.16-2h4.68c.09.65.16 1.32.16 2 0 .68-.07 1.34-.16 2zm.25 5.56c.6-1.11 1.06-2.31 1.38-3.56h2.95c-.96 1.65-2.49 2.93-4.33 3.56zM16.36 14c.08-.66.14-1.32.14-2 0-.68-.06-1.34-.14-2h3.38c.16.64.26 1.31.26 2s-.1 1.36-.26 2h-3.38z',
        link: 'M3.9 12c0-1.71 1.39-3.1 3.1-3.1h4V7H7c-2.76 0-5 2.24-5 5s2.24 5 5 5h4v-1.9H7c-1.71 0-3.1-1.39-3.1-3.1zM8 13h8v-2H8v2zm9-6h-4v1.9h4c1.71 0 3.1 1.39 3.1 3.1s-1.39 3.1-3.1 3.1h-4V17h4c2.76 0 5-2.24 5-5s-2.24-5-5-5z',
        history: 'M13 3a9 9 0 0 0-9 9H1l3.89 3.89.07.14L8.99 12H6c0-3.87 3.13-7 7-7s7 3.13 7 7-3.13 7-7 7c-1.93 0-3.68-.79-4.94-2.06l-1.42 1.42A8.954 8.954 0 0 0 13 21a9 9 0 0 0 0-18zm-1 5v5l4.28 2.54.72-1.21-3.5-2.08V8H12z',
        posts: 'M19 5v14H5V5h14m0-2H5c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h14c1.1 0 2-.9 2-2V5c0-1.1-.9-2-2-2zm-2.5 14H7v-2h9.5v2zm0-4H7v-2h9.5v2zm0-4H7V7h9.5v2z',
        copy: 'M16 1H4c-1.1 0-2 .9-2 2v14h2V3h12V1zm3 4H8c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h11c1.1 0 2-.9 2-2V7c0-1.1-.9-2-2-2zm0 16H8V7h11v14z',
        check: 'M9 16.17L4.83 12l-1.42 1.41L9 19 21 7l-1.41-1.41z',
        flag: 'M14.4 6L14 4H5v17h2v-7h5.6l.4 2h7V6z',
        chart: 'M5 9.2h3V19H5V9.2zm5.6-4.2h2.8v14h-2.8V5zm5.6 8H19v6h-2.8v-6z',
    };
    function svg(name, className) {
        var node = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
        node.setAttribute('class', className || 'i');
        node.setAttribute('viewBox', '0 0 24 24');
        node.setAttribute('fill', 'currentColor');
        node.setAttribute('aria-hidden', 'true');
        var path = document.createElementNS('http://www.w3.org/2000/svg', 'path');
        path.setAttribute('d', ICONS[name]);
        node.appendChild(path);
        return node;
    }
    // The caret the admin's own `<select>`s carry. `.field select` sets
    // `appearance: none` and reserves 40px on the right for it, so a select
    // built here without one is a control with no affordance and a gap where
    // the arrow should be.
    function caret(label) {
        var node = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
        node.setAttribute('class', 'car');
        node.setAttribute('viewBox', '0 0 24 24');
        node.setAttribute('fill', 'currentColor');
        node.setAttribute('aria-hidden', 'true');
        var path = document.createElementNS('http://www.w3.org/2000/svg', 'path');
        path.setAttribute('d', 'M7 10l5 5 5-5z');
        node.appendChild(path);
        label.appendChild(node);
    }

    // A settings card with a section heading above it, as the rest of the
    // admin builds them. `--i` staggers the entrance animation card by card.
    var order = 0;
    function card(title, parent) {
        element('div', title, parent).className = 'section';
        var node = element('div', '', parent);
        node.className = 'card rows rise';
        node.style.setProperty('--i', ++order);
        return node;
    }
    // Every row carries an icon, because the divider between rows is drawn
    // from `left: 76px` — the width of the icon column. A row without one
    // still starts its content at the card's padding, so the rule floats
    // above nothing and the column of titles comes out ragged.
    function row(parent, iconName, tint, top) {
        var node = element('div', '', parent);
        node.className = 'row';
        if (top) node.style.alignItems = 'flex-start';
        var holder = element('span', '', node);
        holder.className = 'ic ' + (tint || 'tint-slate');
        holder.appendChild(svg(iconName || 'tune'));
        var body = element('div', '', node);
        body.className = 'body';
        body.style.width = '100%';
        return {row: node, body: body};
    }
    function trailing(parent) {
        var node = element('div', '', parent);
        node.className = 'trailing';
        return node;
    }
    function heading(parent, title, detail) {
        element('span', title, parent).className = 'title';
        var node = element('span', detail || '', parent);
        node.className = 'detail';
        return node;
    }
    function note(parent, text) {
        var node = element('p', text, parent);
        node.className = 'muted';
        node.style.margin = '10px 0 0';
        return node;
    }
    // The admin's switch: a hidden checkbox driving a track and thumb.
    function toggle(parent, label, checked) {
        var holder = element('label', '', parent);
        holder.className = 'switch';
        var input = element('input', '', holder);
        input.type = 'checkbox';
        input.checked = checked === true;
        input.setAttribute('aria-label', label);
        var track = element('span', '', holder);
        track.className = 'track';
        element('span', '', track).className = 'thumb';
        return input;
    }
    function field(parent, label, value, type) {
        var holder = element('label', '', parent);
        holder.className = 'field';
        holder.style.margin = '0';
        var input = element('input', '', holder);
        input.type = type || 'text';
        input.placeholder = ' ';
        input.value = value == null ? '' : value;
        element('span', label, holder);
        return input;
    }
    function picker(parent, label, choices, value) {
        var holder = element('label', '', parent);
        // `fixed` keeps the floating label up, which is what a select needs:
        // it always has a value, so there is nothing to float for.
        holder.className = 'field fixed';
        holder.style.margin = '0';
        var input = element('select', '', holder);
        choices.forEach(function (pair) {
            element('option', pair[1], input).value = pair[0];
        });
        input.value = value;
        // A stored value outside the list is still shown, the way the
        // server-rendered currency and timezone pickers do it.
        if (input.value !== value && value) {
            var extra = element('option', value, input);
            extra.value = value;
            input.value = value;
        }
        element('span', label, holder);
        caret(holder);
        return input;
    }
    function grid(parent) {
        var node = element('div', '', parent);
        node.className = 'grid2';
        node.style.gap = '12px';
        return node;
    }

    // `created_at` is a UTC database timestamp with no zone marker, so it is
    // stamped as UTC before being shown in the reader's own zone.
    function when(raw) {
        var parsed = new Date(String(raw).replace(' ', 'T') + 'Z');
        if (isNaN(parsed)) return String(raw);
        return parsed.toLocaleString(undefined, {dateStyle: 'medium', timeStyle: 'short'});
    }
    function money(amount, currency) {
        return (amount / 100).toFixed(2) + ' ' + currency;
    }

    var CURRENCIES = [['USD', 'USD $'], ['EUR', 'EUR €'], ['GBP', 'GBP £'], ['AUD', 'AUD $'], ['CAD', 'CAD $'], ['INR', 'INR ₹']];
    var NETWORKS = [['eip155:8453', 'Base mainnet · live Stripe keys'], ['eip155:84532', 'Base Sepolia · test Stripe keys']];

    // Live / Off / Not ready, from the readiness the API reports per method.
    // The reason is the whole point of the card: a method that is switched on
    // but missing a deposit address issues no challenge, and nothing else on
    // the page would say so.
    function status(parent, state) {
        var chip = element('span', '', parent);
        if (state.live) {
            chip.className = 'chip published';
            chip.textContent = 'Live';
        } else if (state.enabled) {
            chip.className = 'chip error';
            chip.textContent = 'Not ready';
        } else {
            chip.className = 'chip tonal';
            chip.textContent = 'Off';
        }
        return chip;
    }
    function banner(parent, kind, iconName, text) {
        var node = element('div', '', parent);
        node.className = 'banner ' + kind + ' rise';
        node.appendChild(svg(iconName));
        var message = element('span', text, node);
        message.className = 'msg';
        return message;
    }

    async function load() {
        var results = await Promise.all([request('settings/'), request('payments/')]);
        var data = results[0], config = data.machine_payments, methods = data.methods;
        var NAMES = {stripe: 'cards', tempo: 'Tempo USDC', x402: 'Base USDC'};
        var keys = ['stripe', 'tempo', 'x402'];
        var live = keys.filter(function (key) { return methods[key].live; });
        var switchedOn = keys.filter(function (key) { return methods[key].enabled; });
        host.replaceChildren();
        order = 0;

        // --- what the site is actually doing right now ------------------
        if (data.is_private) {
            banner(host, 'error', 'flag', 'This publication is private, so machine payments cannot be switched on. Make the site public in Settings › General first.');
        } else if (!data.stripe_connected) {
            banner(host, 'error', 'flag', 'Stripe is not connected. Every payment method settles through Stripe, so connect it in Settings › Membership first.');
        } else if (!config.enabled) {
            banner(host, 'info', 'tune', 'Paid Markdown access is off. Agents asking for a paid article as Markdown get the usual gated response.');
        } else if (!switchedOn.length) {
            banner(host, 'error', 'flag', 'Paid Markdown access is on, but no payment method is switched on — nothing is being offered for sale. Turn on cards, Tempo or Base below.');
        } else if (!live.length) {
            banner(host, 'error', 'flag', 'Paid Markdown access is on, but every payment method that is switched on reads “Not ready” — nothing is being offered for sale. Each card below says what it is waiting for.');
        } else {
            var names = live.map(function (key) { return NAMES[key]; });
            var taking = 'Taking payment by ' + (names.length > 1 ? names.slice(0, -1).join(', ') + ' and ' + names[names.length - 1] : names[0]) + '.';
            if (!data.eligible_posts) {
                banner(host, 'info', 'tune', taking + ' Nothing is for sale yet: only published articles with paid or tier-only visibility can be bought as Markdown.');
            } else {
                banner(host, 'ok', 'check', taking + ' ' + data.eligible_posts + ' paid ' + (data.eligible_posts === 1 ? 'article' : 'articles') + ' can be bought by appending .md to the article URL.');
            }
        }

        var form = element('form', '', host);

        // --- the switch everything else hangs off ------------------------
        var access = card('Access', form);
        var accessRow = row(access, 'tune', 'tint-pink');
        heading(accessRow.body, 'Paid Markdown access', 'Agents can pay to read one paid article as Markdown. No membership is created and no other article is unlocked.');
        var enabled = toggle(trailing(accessRow.row), 'Paid Markdown access', config.enabled);
        if (data.is_private || !data.stripe_connected) enabled.disabled = true;

        var eligibleRow = row(access, 'posts', 'tint-blue');
        heading(eligibleRow.body, 'Articles for sale', data.eligible_posts
            ? 'Published articles set to paid or tier-only visibility.'
            : 'Nothing is for sale yet — set an article’s visibility to paid or specific tiers.');
        var eligibleChip = element('span', String(data.eligible_posts), trailing(eligibleRow.row));
        eligibleChip.className = data.eligible_posts ? 'chip tonal' : 'chip draft';

        if (!data.llms_enabled) {
            var llmsRow = row(access, 'flag', 'tint-amber');
            heading(llmsRow.body, 'Markdown delivery is off', 'The .md representation is part of the LLM switch in Settings › Meta data. Saving with access on turns it back on.');
        }

        // --- price -------------------------------------------------------
        var pricing = card('Pricing', form);
        var priceRow = row(pricing, 'tune', 'tint-purple', true);
        var priceGrid = grid(priceRow.body);
        var price = field(priceGrid, 'Price per article', (config.amount / 100).toFixed(2), 'number');
        price.step = '.01';
        price.min = '.01';
        price.max = '10000';
        var currency = picker(priceGrid, 'Currency', CURRENCIES, config.currency);
        note(priceRow.body, 'Charged once per article, per agent. Stablecoin methods charge the same number in USDC — no conversion is applied.');

        // --- Stripe cards and Link ---------------------------------------
        //
        // Cards have no switch of their own: the profile ID is the switch, so
        // the chip row reads the same as Tempo's and Base's with the control
        // one row below it.
        var cards = card('Stripe cards & Link', form);
        var cardsRow = row(cards, 'lock', 'tint-teal');
        heading(cardsRow.body, 'Accept cards and Link through MPP', methods.stripe.reason || 'Charged once through your connected Stripe account.');
        status(trailing(cardsRow.row), methods.stripe);

        var profileRow = row(cards, 'tune', 'tint-teal', true);
        var profile = field(profileRow.body, 'Stripe business network profile ID', config.stripe_profile_id);
        note(profileRow.body, data.stripe_connected
            ? 'Cards are accepted only while this is filled in, and need a price of at least 0.50 in the chosen currency.'
            : 'Connect Stripe in Settings › Membership before using card payments.');

        // --- Tempo --------------------------------------------------------
        var tempoCard = card('Tempo USDC', form);
        var tempoRow = row(tempoCard, 'globe', 'tint-green');
        heading(tempoRow.body, 'Accept Tempo USDC through MPP', methods.tempo.reason || 'Settles on the Tempo network through the Merchant Payment Protocol.');
        var tempoTrailing = trailing(tempoRow.row);
        status(tempoTrailing, methods.tempo);
        var tempo = toggle(tempoTrailing, 'Accept Tempo USDC', config.tempo_enabled);

        var tempoNetRow = row(tempoCard, 'tune', 'tint-green');
        heading(tempoNetRow.body, 'Use the Tempo testnet', 'Pairs with Stripe test keys. Live Stripe keys need this off, and test keys need it on.');
        var tempoTestnet = toggle(trailing(tempoNetRow.row), 'Use the Tempo testnet', config.tempo_testnet);

        deposit(tempoCard, 'tempo', 'Tempo deposit address', data.deposit_addresses.tempo);

        // --- Base / x402 ----------------------------------------------------
        var baseCard = card('Base USDC · x402', form);
        var baseRow = row(baseCard, 'globe', 'tint-amber');
        heading(baseRow.body, 'Accept Base USDC through x402', methods.x402.reason || 'Settles on Base through the configured public facilitator.');
        var baseTrailing = trailing(baseRow.row);
        status(baseTrailing, methods.x402);
        var x402 = toggle(baseTrailing, 'Accept Base USDC', config.x402_enabled);

        var baseNetRow = row(baseCard, 'tune', 'tint-amber', true);
        var network = picker(baseNetRow.body, 'Settlement network', NETWORKS, config.x402_network);
        note(baseNetRow.body, 'Mainnet needs live Stripe keys; Sepolia needs test keys. A mismatch keeps x402 switched off.');

        var facilitatorRow = row(baseCard, 'link', 'tint-amber', true);
        var facilitator = field(facilitatorRow.body, 'Settlement facilitator', config.x402_facilitator, 'url');
        note(facilitatorRow.body, 'Must be an HTTPS origin. The x402.org facilitator is testnet only.');

        deposit(baseCard, 'base', 'Base deposit address', data.deposit_addresses.base);

        // --- actions --------------------------------------------------------
        var actions = element('div', '', form);
        actions.style.cssText = 'display:flex;align-items:center;gap:12px;flex-wrap:wrap;margin:20px 0 28px';
        var save = element('button', 'Save machine payments', actions);
        save.className = 'btn';
        save.type = 'submit';
        var message = element('p', '', actions);
        message.setAttribute('role', 'status');
        message.className = 'muted';
        message.style.margin = '0';

        // Fields belonging to a method that is switched off stay visible but
        // inert, so the card still documents what the method needs without
        // inviting edits that change nothing.
        function sync() {
            tempoTestnet.disabled = !tempo.checked;
            network.disabled = facilitator.disabled = !x402.checked;
            [tempoNetRow.row, baseNetRow.row, facilitatorRow.row].forEach(function (node) {
                var off = node === tempoNetRow.row ? !tempo.checked : !x402.checked;
                node.style.opacity = off ? '.55' : '';
            });
        }
        tempo.onchange = x402.onchange = sync;
        sync();

        form.onsubmit = async function (event) {
            event.preventDefault();
            save.disabled = true;
            message.className = 'muted';
            try {
                var amount = Number(price.value) * 100;
                if (!Number.isFinite(amount) || Math.abs(amount - Math.round(amount)) > 0.000001) throw new Error('Use no more than two decimal places.');
                await request('settings/', 'PUT', {
                    enabled: enabled.checked,
                    amount: Math.round(amount),
                    currency: currency.value.trim().toUpperCase(),
                    stripe_profile_id: profile.value.trim(),
                    tempo_enabled: tempo.checked,
                    tempo_testnet: tempoTestnet.checked,
                    x402_enabled: x402.checked,
                    x402_network: network.value,
                    x402_facilitator: facilitator.value.trim(),
                });
                // Re-read rather than just reporting success: the readiness
                // chips and the banner are server-side judgements, and leaving
                // them stale is how "it saved but nothing happens" starts.
                await load();
            } catch (error) {
                message.className = 'muted';
                message.style.color = 'var(--error)';
                message.textContent = error.message;
                save.disabled = false;
            }
        };

        // --- what agents have actually paid ------------------------------
        var activity = card('Payment activity', host);
        var payments = results[1].payments;
        if (!payments.length) {
            var emptyRow = row(activity, 'history', 'tint-slate');
            heading(emptyRow.body, 'No payment attempts yet', 'Every challenge an agent answers is recorded here, settled or not.');
        }
        payments.forEach(function (payment) {
            var entry = row(activity, 'history', payment.status === 'settled' ? 'tint-green' : payment.status === 'failed' ? 'tint-red' : 'tint-slate');
            element('span', payment.title || payment.post_id, entry.body).className = 'title';
            var detail = element('span', '', entry.body);
            detail.className = 'detail';
            detail.textContent = when(payment.created_at) + ' · ' + payment.protocol + '/' + payment.method;
            if (payment.reference) {
                detail.appendChild(document.createTextNode(' · '));
                element('code', payment.reference, detail);
            }
            var end = trailing(entry.row);
            element('span', money(payment.amount, payment.currency), end).className = 'detail';
            var chip = element('span', payment.status, end);
            chip.className = 'chip ' + (payment.status === 'settled' ? 'published' : payment.status === 'failed' ? 'error' : 'draft');
        });
    }

    // A deposit address row: the address itself when Stripe has issued one,
    // otherwise the button that asks Stripe for it. `data-copy` is handled by
    // the admin's own delegated copy listener, toast included.
    function deposit(parent, network, label, address) {
        var entry = row(parent, 'globe', 'tint-slate', !!address);
        if (address) {
            element('span', label, entry.body).className = 'title';
            var key = element('span', '', entry.body);
            key.className = 'key';
            key.style.cssText = 'margin-top:6px;max-width:100%;word-break:break-all';
            element('code', address, key);
            var copy = element('button', '', key);
            copy.type = 'button';
            copy.title = 'Copy';
            copy.setAttribute('aria-label', 'Copy ' + label);
            copy.dataset.copy = address;
            copy.appendChild(svg('copy'));
            return;
        }
        heading(entry.body, label, 'Stripe issues the address this network settles into. Funds arrive in your Stripe balance.');
        var button = element('button', 'Create address', trailing(entry.row));
        button.type = 'button';
        button.className = 'btn tonal small';
        button.onclick = async function () {
            button.disabled = true;
            button.textContent = 'Creating…';
            try {
                await request('deposit-addresses/', 'POST', {network: network});
                await load();
            } catch (error) {
                button.disabled = false;
                button.textContent = 'Create address';
                var failure = entry.body.querySelector('.detail');
                failure.textContent = error.message;
                failure.style.color = 'var(--error)';
            }
        };
    }

    // The host is a bare container now that the page builds its own cards, so
    // a failure has to bring its own card or it renders as loose text on the
    // page background.
    load().catch(function (error) {
        host.replaceChildren();
        var holder = element('div', '', host);
        holder.className = 'card padded';
        var text = element('p', error.message, holder);
        text.setAttribute('role', 'status');
        text.className = 'muted';
        text.style.margin = '0';
    });
})();
