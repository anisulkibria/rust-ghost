(function () {
    'use strict';
    var host = document.getElementById('checkout-design');
    if (!host) return;
    var base = location.pathname.split('/ghost/')[0] + '/ghost/api/admin/';
    async function request(path, method, body) {
        var response = await fetch(base + path, {method: method || 'GET', credentials: 'same-origin', cache: 'no-store', headers: {'Content-Type': 'application/json'}, body: body === undefined ? undefined : JSON.stringify(body)});
        var data = await response.json();
        if (!response.ok) throw new Error(data.errors && data.errors[0] && data.errors[0].message || 'The request failed');
        return data;
    }
    function element(tag, text, parent) {
        var node = document.createElement(tag);
        if (text) node.textContent = text;
        if (parent) parent.appendChild(node);
        return node;
    }
    // The caret the admin's own `<select>`s carry. `.field select` sets
    // `appearance: none` and reserves 40px on the right for it, so a select
    // built here without one is a control with no affordance and a gap where
    // the arrow should be.
    function caret(label) {
        var svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
        svg.setAttribute('class', 'car');
        svg.setAttribute('viewBox', '0 0 24 24');
        svg.setAttribute('fill', 'currentColor');
        svg.setAttribute('aria-hidden', 'true');
        var path = document.createElementNS('http://www.w3.org/2000/svg', 'path');
        path.setAttribute('d', 'M7 10l5 5 5-5z');
        svg.appendChild(path);
        label.appendChild(svg);
        return svg;
    }

    // A settings card with a section heading above it, as the rest of the
    // admin builds them.
    function card(title, parent) {
        element('div', title, parent).className = 'section';
        var node = element('div', '', parent);
        node.className = 'card rows';
        return node;
    }

    // Material icon paths, by the names `svg()` uses server-side. Only the
    // handful this page needs — duplicating the whole set here would be a
    // second copy to keep in step.
    var ICONS = {
        tune: 'M3 17v2h6v-2H3zM3 5v2h10V5H3zm10 16v-2h8v-2h-8v-2h-2v6h2zM7 9v2H3v2h4v2h2V9H7zm14 4v-2H11v2h10zm-6-4h2V7h4V5h-4V3h-2v6z',
        themes: 'M12 3c-4.97 0-9 4.03-9 9s4.03 9 9 9c.83 0 1.5-.67 1.5-1.5 0-.39-.15-.74-.39-1.01-.23-.26-.38-.61-.38-.99 0-.83.67-1.5 1.5-1.5H16c2.76 0 5-2.24 5-5 0-4.42-4.03-8-9-8zm-5.5 9c-.83 0-1.5-.67-1.5-1.5S5.67 8 6.5 8 8 8.67 8 9.5 7.33 12 6.5 12zm3-4C8.67 8 8 7.33 8 6.5S8.67 5 9.5 5s1.5.67 1.5 1.5S10.33 8 9.5 8zm5 0c-.83 0-1.5-.67-1.5-1.5S13.67 5 14.5 5s1.5.67 1.5 1.5S15.33 8 14.5 8zm3 4c-.83 0-1.5-.67-1.5-1.5S16.67 9 17.5 9s1.5.67 1.5 1.5-.67 1.5-1.5 1.5z',
        title: 'M5 4v3h5.5v12h3V7H19V4z',
        image: 'M21 19V5c0-1.1-.9-2-2-2H5c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h14c1.1 0 2-.9 2-2zM8.5 13.5l2.5 3.01L14.5 12l4.5 6H5l3.5-4.5z',
    };
    function icon(name, tint, parent) {
        var holder = element('span', '', parent);
        holder.className = 'ic ' + tint;
        var svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
        svg.setAttribute('class', 'i');
        svg.setAttribute('viewBox', '0 0 24 24');
        svg.setAttribute('fill', 'currentColor');
        svg.setAttribute('aria-hidden', 'true');
        var path = document.createElementNS('http://www.w3.org/2000/svg', 'path');
        path.setAttribute('d', ICONS[name]);
        svg.appendChild(path);
        holder.appendChild(svg);
        return holder;
    }

    // Every row carries an icon, because the divider between rows is drawn
    // from `left: 76px` — the width of the icon column. A row without one
    // still starts its content at the card's padding, so the rule floats
    // above nothing and the column of titles comes out ragged.
    function row(parent, iconName, tint) {
        var node = element('div', '', parent);
        node.className = 'row';
        icon(iconName || 'tune', tint || 'tint-slate', node);
        return node;
    }

    async function load() {
        var results = await Promise.all([request('stripe/checkout/config/'), request('tiers/?limit=all')]);
        var config = results[0], design = config.checkout_config[0].design;
        host.replaceChildren();

        var form = element('form', '', host);
        var fields = {};

        // --- the switch that turns the rest on -------------------------
        var appearance = card('Appearance', form);
        var toggleRow = row(appearance, 'tune', 'tint-pink');
        var toggleBody = element('div', '', toggleRow);
        toggleBody.className = 'body';
        element('span', 'Customize checkout appearance', toggleBody).className = 'title';
        element('span', 'Off uses Stripe’s own default checkout styling.', toggleBody).className = 'detail';
        var trailing = element('div', '', toggleRow);
        trailing.className = 'trailing';
        var switchLabel = element('label', '', trailing);
        switchLabel.className = 'switch';
        var enabled = element('input', '', switchLabel);
        enabled.type = 'checkbox';
        enabled.checked = design.customize === true;
        enabled.setAttribute('aria-label', 'Customize checkout appearance');
        var track = element('span', '', switchLabel);
        track.className = 'track';
        element('span', '', track).className = 'thumb';

        // --- colours ---------------------------------------------------
        //
        // Through `.color-field`, the admin's existing swatch: a 44px round
        // well beside the name and the current value. A colour input dropped
        // into `.field` instead inherits `padding: 22px 14px 6px` and
        // `width: 100%`, which rendered as a 1032px-wide, 30px-tall strip
        // with the swatch crushed inside it.
        function colour(key, title, fallback, tint) {
            var holder = row(appearance, 'themes', tint);
            var body = element('div', '', holder);
            body.className = 'body';
            var field = element('div', '', body);
            field.className = 'color-field';
            var input = element('input', '', field);
            input.type = 'color';
            input.value = design[key] || fallback;
            input.setAttribute('aria-label', title);
            var text = element('div', '', field);
            var name = element('span', title, text);
            name.className = 'title';
            name.style.display = 'block';
            var hex = element('span', input.value, text);
            hex.className = 'detail';
            // The value is the only way to read a colour back out, so it
            // tracks the picker rather than showing whatever it was on load.
            input.addEventListener('input', function () { hex.textContent = input.value; });
            fields[key] = input;
        }
        colour('button_color', 'Button colour', '#15171a', 'tint-purple');
        colour('background_color', 'Background colour', '#ffffff', 'tint-blue');

        // --- the two selects -------------------------------------------
        function choose(key, title, choices, fallback, iconName, tint) {
            var holder = row(appearance, iconName, tint);
            var body = element('div', '', holder);
            body.className = 'body';
            var label = element('label', '', body);
            // `fixed` keeps the floating label up, which is what a select
            // needs: it always has a value, so there is nothing to float for.
            label.className = 'field fixed';
            label.style.cssText = 'margin:0;max-width:320px';
            var input = element('select', '', label);
            choices.forEach(function (pair) {
                var option = element('option', pair[1], input);
                option.value = pair[0];
            });
            input.value = design[key] || fallback;
            element('span', title, label);
            caret(label);
            fields[key] = input;
        }
        choose('border_style', 'Button corners', [['rounded', 'Rounded'], ['rectangular', 'Square'], ['pill', 'Pill']], 'rounded', 'image', 'tint-amber');
        choose('font_family', 'Font', config.fonts.map(function (font) {
            return [font, font === 'default' ? 'Stripe default' : font.split('_').map(function (word) {
                return word[0].toUpperCase() + word.slice(1);
            }).join(' ')];
        }), 'default', 'title', 'tint-teal');

        function value() {
            var result = {customize: enabled.checked};
            Object.keys(fields).forEach(function (key) { result[key] = fields[key].value; });
            return {design: result};
        }

        // --- preview ----------------------------------------------------
        var paid = results[1].tiers.filter(function (tier) { return tier.type === 'paid' && tier.active !== false; });
        var plans = null;
        if (paid.length) {
            var previewCard = card('Preview', form);
            var previewRow = row(previewCard, 'themes', 'tint-green');
            var previewBody = element('div', '', previewRow);
            previewBody.className = 'body';
            var planLabel = element('label', '', previewBody);
            planLabel.className = 'field fixed';
            planLabel.style.cssText = 'margin:0;max-width:320px';
            plans = element('select', '', planLabel);
            paid.forEach(function (tier) {
                [['month', tier.monthly_price], ['year', tier.yearly_price]].forEach(function (plan) {
                    if (!plan[1]) return;
                    var option = element('option', tier.name + ' · ' + (plan[0] === 'month' ? 'Monthly' : 'Yearly'), plans);
                    option.value = tier.id + '/' + plan[0];
                });
            });
            element('span', 'Tier and billing period', planLabel);
            caret(planLabel);
            element('p', 'Opens a real Stripe Checkout session with these settings. Preview payments do not grant memberships.', previewBody).className = 'muted';
        }

        // --- actions ----------------------------------------------------
        //
        // One action bar for the whole form, below both cards: a save button
        // sitting inside the appearance card and a preview button inside the
        // preview card read as two unrelated forms.
        var actions = element('div', '', form);
        actions.className = 'actions';
        actions.style.cssText = 'display:flex;align-items:center;gap:12px;flex-wrap:wrap;margin-top:20px';
        var save = element('button', 'Save checkout design', actions);
        save.className = 'btn';
        save.type = 'submit';
        var preview = null;
        if (plans) {
            preview = element('button', 'Create preview', actions);
            preview.type = 'button';
            preview.className = 'btn tonal';
        }
        var message = element('p', '', actions);
        message.setAttribute('role', 'status');
        message.className = 'muted';
        message.style.margin = '0';

        form.onsubmit = async function (event) {
            event.preventDefault();
            save.disabled = true;
            try {
                await request('stripe/checkout/config/', 'PUT', value());
                message.textContent = 'Checkout design saved';
            } catch (error) {
                message.textContent = error.message;
            } finally {
                save.disabled = false;
            }
        };

        if (preview) {
            preview.onclick = async function () {
                if (!plans.value) return;
                preview.disabled = true;
                try {
                    var selected = plans.value.split('/'), body = value();
                    body.tier_id = selected[0];
                    body.cadence = selected[1];
                    var result = await request('stripe/checkout/preview/', 'POST', body);
                    message.replaceChildren();
                    var link = element('a', 'Open checkout preview', message);
                    link.href = result.checkout_preview[0].url;
                    link.target = '_blank';
                    link.rel = 'noopener noreferrer';
                } catch (error) {
                    message.textContent = error.message;
                } finally {
                    preview.disabled = false;
                }
            };
        }
    }
    // The host is a bare container now that the form builds its own cards, so
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
