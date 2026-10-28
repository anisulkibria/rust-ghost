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
    async function load() {
        var results = await Promise.all([request('stripe/checkout/config/'), request('tiers/?limit=all')]);
        var config = results[0], design = config.checkout_config[0].design;
        host.replaceChildren();
        var form = element('form', '', host), message = element('p', '', host);
        message.setAttribute('role', 'status');
        var enabledLabel = element('label', '', form);
        enabledLabel.style.cssText = 'display:flex;align-items:center;gap:12px;margin-bottom:24px';
        var enabled = element('input', '', enabledLabel);
        enabled.type = 'checkbox'; enabled.checked = design.customize === true;
        element('span', 'Customize checkout appearance', enabledLabel);
        var fields = {};
        function field(key, title, choices, fallback) {
            var label = element('label', '', form); label.className = 'field fixed'; label.style.display = 'block';
            var input = element(choices ? 'select' : 'input', '', label);
            if (choices) choices.forEach(function (pair) { var option = element('option', pair[1], input); option.value = pair[0]; });
            else input.type = 'color';
            input.value = design[key] || fallback;
            element('span', title, label); fields[key] = input;
            return input;
        }
        field('button_color', 'Button color', null, '#15171a');
        field('background_color', 'Background color', null, '#ffffff');
        field('border_style', 'Button corners', [['rounded', 'Rounded'], ['rectangular', 'Square'], ['pill', 'Pill']], 'rounded');
        field('font_family', 'Font', config.fonts.map(function (font) { return [font, font === 'default' ? 'Stripe default' : font.split('_').map(function (word) { return word[0].toUpperCase() + word.slice(1); }).join(' ')]; }), 'default');
        function value() { var result = {customize: enabled.checked}; Object.keys(fields).forEach(function (key) { result[key] = fields[key].value; }); return {design: result}; }
        var save = element('button', 'Save checkout design', form); save.className = 'btn'; save.type = 'submit';
        form.onsubmit = async function (event) {
            event.preventDefault(); save.disabled = true;
            try { await request('stripe/checkout/config/', 'PUT', value()); message.textContent = 'Checkout design saved'; }
            catch (error) { message.textContent = error.message; }
            finally { save.disabled = false; }
        };
        var paid = results[1].tiers.filter(function (tier) { return tier.type === 'paid' && tier.active !== false; });
        if (paid.length) {
            element('h3', 'Preview', form);
            var plans = element('select', '', form);
            paid.forEach(function (tier) { [['month', tier.monthly_price], ['year', tier.yearly_price]].forEach(function (plan) {
                if (!plan[1]) return;
                var option = element('option', tier.name + ' · ' + (plan[0] === 'month' ? 'Monthly' : 'Yearly'), plans);
                option.value = tier.id + '/' + plan[0];
            }); });
            var preview = element('button', 'Create preview', form); preview.type = 'button'; preview.className = 'btn tonal';
            preview.onclick = async function () {
                if (!plans.value) return;
                preview.disabled = true;
                try {
                    var selected = plans.value.split('/'), body = value(); body.tier_id = selected[0]; body.cadence = selected[1];
                    var result = await request('stripe/checkout/preview/', 'POST', body);
                    message.replaceChildren(); var link = element('a', 'Open checkout preview', message);
                    link.href = result.checkout_preview[0].url; link.target = '_blank'; link.rel = 'noopener noreferrer';
                } catch (error) { message.textContent = error.message; }
                finally { preview.disabled = false; }
            };
            element('p', 'Preview uses Stripe Checkout. Preview payments do not grant memberships.', form).className = 'muted';
        }
    }
    load().catch(function (error) { host.textContent = error.message; });
})();
