/* Portfolio filters, gallery and print actions are handled by Vuuut. */
(function () {
  'use strict';

  var drawer = document.querySelector('[data-sl-drawer]');
  var burger = document.querySelector('[data-sl-burger]');
  if (drawer && burger) {
    var close = drawer.querySelector('[data-sl-drawer-close]');
    burger.addEventListener('click', function () {
      drawer.showModal();
      burger.setAttribute('aria-expanded', 'true');
    });
    close.addEventListener('click', function () { drawer.close(); });
    drawer.addEventListener('close', function () {
      burger.setAttribute('aria-expanded', 'false');
      burger.focus();
    });
  }

  var index = document.querySelector('[data-sl-index]');
  var preview = index && index.querySelector('[data-sl-peek]');
  if (preview && window.matchMedia('(hover: hover) and (pointer: fine) and (min-width: 861px)').matches) {
    function position(event) {
      var halfWidth = preview.offsetWidth / 2;
      var halfHeight = preview.offsetHeight / 2;
      var x = Math.min(window.innerWidth - halfWidth - 16, event.clientX + halfWidth + 32);
      var y = Math.max(halfHeight + 16, Math.min(window.innerHeight - halfHeight - 16, event.clientY));
      preview.style.left = x + 'px';
      preview.style.top = y + 'px';
    }

    index.querySelectorAll('[data-sl-preview]').forEach(function (row) {
      row.addEventListener('pointerenter', function (event) {
        var source = row.getAttribute('data-sl-preview');
        if (!source) return;
        preview.src = source;
        preview.hidden = false;
        position(event);
        preview.classList.add('on');
      });
      row.addEventListener('pointermove', position);
      row.addEventListener('pointerleave', function () { preview.classList.remove('on'); });
    });
  }
})();
