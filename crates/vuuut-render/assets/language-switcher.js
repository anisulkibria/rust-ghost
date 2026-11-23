/* Vuuut language switcher.
 *
 * Injected by {{ghost_head}}, like Portal, on any page whose menu carries the
 * language switcher. The theme renders the switcher as one ordinary menu link
 * (labelled with the current language); this finds that link — by where it
 * points and what it says, because a theme's menu markup carries no class we
 * chose — and turns it into a dropdown of every language. No theme change.
 *
 * The menu is one fixed-position layer on <body>, so a header with
 * overflow:hidden, a sticky bar or a theme's "more" menu cannot clip it, and
 * every click is delegated from the document, so it keeps working when a
 * theme moves or clones the link (Source does, when its menu overflows).
 * Without this script the link still goes to the next language.
 */
(function () {
  'use strict';
  var node = document.getElementById('vuuut-language-data');
  if (!node) { return; }
  var data;
  try { data = JSON.parse(node.textContent); } catch (e) { return; }
  if (!data || !data.trigger || !data.languages || data.languages.length < 2) { return; }

  var target = new URL(data.trigger.url, location.href).href;
  var label = String(data.trigger.label).trim();
  var menu = null;
  var openFor = null;

  var css =
    '.vl-toggle{display:inline-flex!important;align-items:center;gap:.4em;cursor:pointer}' +
    '.vl-caret{display:inline-block;width:.42em;height:.42em;border-right:1.5px solid currentColor;' +
    'border-bottom:1.5px solid currentColor;transform:translateY(-30%) rotate(45deg);transition:transform .15s ease;flex:none}' +
    '.vl-toggle[aria-expanded="true"] .vl-caret{transform:translateY(20%) rotate(-135deg)}' +
    '.vl-menu{position:fixed;z-index:2147483000;min-width:190px;max-width:calc(100vw - 16px);max-height:calc(100vh - 16px);' +
    'overflow:auto;box-sizing:border-box;margin:0;padding:6px;list-style:none;border-radius:10px;' +
    'background:#fff;color:#15171a;font-size:15px;line-height:1.35;text-align:left;letter-spacing:normal;text-transform:none;' +
    'box-shadow:0 0 0 1px rgba(0,0,0,.07),0 12px 32px -8px rgba(0,0,0,.28);' +
    'opacity:0;visibility:hidden;transform:translateY(-4px);transition:opacity .14s ease,transform .14s ease,visibility 0s .14s}' +
    '.vl-menu.vl-open{opacity:1;visibility:visible;transform:none;transition:opacity .14s ease,transform .14s ease}' +
    '.vl-menu.vl-dark{background:#1f2125;color:#f1f2f4;box-shadow:0 0 0 1px rgba(255,255,255,.1),0 12px 32px -8px rgba(0,0,0,.6)}' +
    '.vl-menu li{margin:0;padding:0;list-style:none}' +
    '.vl-menu a{display:flex;align-items:center;justify-content:space-between;gap:18px;padding:8px 12px;border-radius:7px;' +
    'color:inherit!important;text-decoration:none!important;font-weight:400;white-space:nowrap}' +
    '.vl-menu a:hover,.vl-menu a:focus-visible{background:rgba(127,127,127,.14);outline:none}' +
    '.vl-menu a[aria-current="true"]{font-weight:600}' +
    '.vl-menu a[aria-current="true"]::after{content:"";width:.32em;height:.64em;flex:none;margin-right:2px;' +
    'border-right:2px solid var(--ghost-accent-color,currentColor);border-bottom:2px solid var(--ghost-accent-color,currentColor);' +
    'transform:translateY(-15%) rotate(45deg)}' +
    '.vl-code{opacity:.55;font-size:.8em;text-transform:uppercase;letter-spacing:.04em;margin-left:auto}' +
    '.vl-menu a[aria-current="true"] .vl-code{display:none}' +
    '@media (prefers-reduced-motion:reduce){.vl-menu,.vl-caret{transition:none}}';

  function addStyles() {
    if (document.getElementById('vuuut-language-styles')) { return; }
    var style = document.createElement('style');
    style.id = 'vuuut-language-styles';
    style.textContent = css;
    document.head.appendChild(style);
  }

  // Is this one of the theme's renderings of the switcher link?
  function isTrigger(a) {
    return a.href === target && (a.textContent || '').trim() === label;
  }

  function enhance() {
    var links = document.querySelectorAll('a[href]');
    for (var i = 0; i < links.length; i++) {
      var a = links[i];
      if (a.classList.contains('vl-toggle') || !isTrigger(a)) { continue; }
      a.classList.add('vl-toggle');
      a.setAttribute('role', 'button');
      a.setAttribute('aria-haspopup', 'true');
      a.setAttribute('aria-expanded', 'false');
      a.setAttribute('lang', data.current);
      var caret = document.createElement('span');
      caret.className = 'vl-caret';
      caret.setAttribute('aria-hidden', 'true');
      a.appendChild(caret);
    }
  }

  function buildMenu() {
    if (menu) { return menu; }
    menu = document.createElement('ul');
    menu.className = 'vl-menu';
    menu.id = 'vuuut-language-menu';
    menu.setAttribute('role', 'menu');
    data.languages.forEach(function (lang) {
      var li = document.createElement('li');
      li.setAttribute('role', 'none');
      var a = document.createElement('a');
      a.href = lang.url;
      a.setAttribute('role', 'menuitem');
      a.setAttribute('hreflang', lang.locale);
      a.setAttribute('lang', lang.locale);
      if (lang.active) { a.setAttribute('aria-current', 'true'); }
      var name = document.createElement('span');
      name.textContent = lang.label;
      var code = document.createElement('span');
      code.className = 'vl-code';
      code.textContent = lang.locale;
      a.appendChild(name);
      a.appendChild(code);
      li.appendChild(a);
      menu.appendChild(li);
    });
    document.body.appendChild(menu);
    return menu;
  }

  // The menu follows the site's own background rather than the OS setting,
  // which a theme may or may not honour.
  function isDark() {
    var el = document.body;
    while (el) {
      var bg = getComputedStyle(el).backgroundColor;
      var m = bg && bg.match(/rgba?\(([\d.]+)[,\s]+([\d.]+)[,\s]+([\d.]+)(?:[,\s/]+([\d.]+))?/);
      if (m && (m[4] === undefined || parseFloat(m[4]) > 0.5)) {
        var lum = 0.2126 * m[1] + 0.7152 * m[2] + 0.0722 * m[3];
        return lum < 110;
      }
      el = el.parentElement === document.documentElement ? null : el.parentElement;
    }
    var root = getComputedStyle(document.documentElement).backgroundColor;
    var r = root && root.match(/rgba?\(([\d.]+)[,\s]+([\d.]+)[,\s]+([\d.]+)/);
    return r ? 0.2126 * r[1] + 0.7152 * r[2] + 0.0722 * r[3] < 110 : false;
  }

  function position(toggle) {
    var rect = toggle.getBoundingClientRect();
    var width = menu.offsetWidth;
    var height = menu.offsetHeight;
    var left = rect.left;
    if (left + width > window.innerWidth - 8) { left = Math.max(8, rect.right - width); }
    var top = rect.bottom + 8;
    if (top + height > window.innerHeight - 8 && rect.top - height - 8 > 8) { top = rect.top - height - 8; }
    menu.style.left = Math.round(left) + 'px';
    menu.style.top = Math.round(top) + 'px';
  }

  function open(toggle) {
    buildMenu();
    menu.classList.toggle('vl-dark', isDark());
    openFor = toggle;
    toggle.setAttribute('aria-expanded', 'true');
    toggle.setAttribute('aria-controls', menu.id);
    position(toggle);
    menu.classList.add('vl-open');
  }

  function close(returnFocus) {
    if (!openFor) { return; }
    var toggle = openFor;
    openFor = null;
    toggle.setAttribute('aria-expanded', 'false');
    if (menu) { menu.classList.remove('vl-open'); }
    if (returnFocus) { toggle.focus(); }
  }

  function items() {
    return menu ? Array.prototype.slice.call(menu.querySelectorAll('a')) : [];
  }

  function focusItem(index) {
    var list = items();
    if (!list.length) { return; }
    list[(index + list.length) % list.length].focus();
  }

  document.addEventListener('click', function (e) {
    var toggle = e.target.closest && e.target.closest('.vl-toggle');
    if (toggle) {
      e.preventDefault();
      // Some themes open their mobile menu on any click inside the nav.
      e.stopPropagation();
      if (openFor === toggle) { close(false); } else { close(false); open(toggle); }
      return;
    }
    if (openFor && !(menu && menu.contains(e.target))) { close(false); }
  }, true);

  document.addEventListener('keydown', function (e) {
    var toggle = e.target.closest && e.target.closest('.vl-toggle');
    if (toggle && (e.key === 'Enter' || e.key === ' ' || e.key === 'ArrowDown')) {
      e.preventDefault();
      if (openFor !== toggle) { close(false); open(toggle); }
      var current = items().findIndex(function (a) { return a.getAttribute('aria-current') === 'true'; });
      focusItem(e.key === 'ArrowDown' ? 0 : Math.max(0, current));
      return;
    }
    if (!openFor) { return; }
    var list = items();
    var at = list.indexOf(document.activeElement);
    if (e.key === 'Escape') { e.preventDefault(); close(true); }
    else if (e.key === 'ArrowDown') { e.preventDefault(); focusItem(at + 1); }
    else if (e.key === 'ArrowUp') { e.preventDefault(); focusItem(at - 1); }
    else if (e.key === 'Home') { e.preventDefault(); focusItem(0); }
    else if (e.key === 'End') { e.preventDefault(); focusItem(list.length - 1); }
    else if (e.key === 'Tab') { close(false); }
  });

  window.addEventListener('resize', function () { close(false); });
  window.addEventListener('scroll', function () { if (openFor) { position(openFor); } }, { passive: true });

  addStyles();
  enhance();
  // A theme that builds or rebuilds its menu after load (a mobile drawer, an
  // overflow menu) gets its copy of the link enhanced too.
  document.addEventListener('DOMContentLoaded', enhance);
  window.addEventListener('load', enhance);
})();
