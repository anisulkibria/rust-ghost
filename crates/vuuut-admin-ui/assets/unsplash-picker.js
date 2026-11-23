/* Shared Unsplash picker for post feature images and publication covers. */
(function () {
  var uHeaders = null;
  var uOnSelect = null;
  function iconSvg(path) {
    return '<svg class="i" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="' + path + '"/></svg>';
  }
  /* ------------------------------------------------- the unsplash picker
   *
   * A vanilla twin of the picker Ghost's editor opens (kg-unsplash's
   * UnsplashSearchModal), down to its behaviour: the latest photos are
   * already on the canvas when it opens, the search box searches live
   * (debounced, from the third character) and clears back to the latest
   * feed, scrolling to the bottom brings the next page from the Link
   * header, a tile hovers up the photographer's credit with a like count
   * and a download, clicking one zooms it, and Insert writes the same
   * URL and credit caption Ghost's does.
   */

  var uModal = null;
  var uNext = null;      // the Link header's rel="next", when there is one
  var uLoading = false;
  var uZoomed = null;
  var uSeq = 0;          // a search in flight answers to its own number
  var uController = null;
  var uSearchTimer = null;

  // Unsplash's guidelines ask for this referral on every link back to them.
  function uReferral(href) {
    try {
      var url = new URL(href);
      url.searchParams.set('utm_source', 'ghost');
      url.searchParams.set('utm_medium', 'referral');
      url.searchParams.set('utm_campaign', 'api-credit');
      return url.href;
    } catch (err) {
      return href;
    }
  }

  function uEsc(text) {
    return String(text == null ? '' : text).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
  }

  var U_HEART = iconSvg('M12 21.35l-1.45-1.32C5.4 15.36 2 12.28 2 8.5 2 5.42 4.42 3 7.5 3c1.74 0 3.41.81 4.5 2.09C13.09 3.81 14.76 3 16.5 3 19.58 3 22 5.42 22 8.5c0 3.78-3.4 6.86-8.55 11.54L12 21.35z');
  var U_DOWNLOAD = iconSvg('M19 9h-4V3H9v6H5l7 7 7-7zM5 18v2h14v-2H5z');

  function unsplashModal() {
    if (uModal) { return uModal; }
    uModal = document.createElement('dialog');
    uModal.className = 'unsplash';
    uModal.setAttribute('aria-label', 'Choose a photo from Unsplash');
    uModal.innerHTML =
      '<header class="u-head"><h3>Unsplash</h3>' +
      '<div class="u-searchbox">' + iconSvg('M15.5 14h-.79l-.28-.27C15.41 12.59 16 11.11 16 9.5 16 5.91 13.09 3 9.5 3S3 5.91 3 9.5 5.91 16 9.5 16c1.61 0 3.09-.59 4.23-1.57l.27.28v.79l5 4.99L20.49 19l-4.99-5zm-6 0C7.01 14 5 11.99 5 9.5S7.01 5 9.5 5 14 7.01 14 9.5 11.99 14 9.5 14z') +
      '<input type="search" placeholder="Search free high-resolution photos" aria-label="Search Unsplash"></div>' +
      '<button class="u-close" type="button" aria-label="Close">' + iconSvg('M19 6.41L17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z') + '</button></header>' +
      '<div class="u-body"><div class="u-grid"></div><div class="u-zoom" hidden></div>' +
      '<div class="u-loading" hidden><i></i></div></div>';
    document.body.appendChild(uModal);

    var input = uModal.querySelector('input');
    // Ghost's box searches live, 300ms after the last keystroke, from the
    // third character; emptying it goes back to the latest feed.
    input.addEventListener('input', function () {
      clearTimeout(uSearchTimer);
      uSearchTimer = setTimeout(function () {
        if (input.value.trim().length > 2) {
          uLoad('https://api.unsplash.com/search/photos?per_page=30&query=' + encodeURIComponent(input.value.trim()), true);
        } else if (!input.value.trim()) {
          uLoad('https://api.unsplash.com/photos?per_page=30', true);
        }
      }, 300);
    });

    // Scrolling to a screenful of the end brings the next page in.
    var body = uModal.querySelector('.u-body');
    body.addEventListener('scroll', function () {
      if (uZoomed || !uNext || uLoading) { return; }
      if (body.scrollTop + body.clientHeight >= body.scrollHeight - 1000) {
        uLoad(uNext, false);
      }
    });

    // Escape steps out of a zoom before it closes the picker, as Ghost's does.
    uModal.addEventListener('cancel', function (event) {
      if (uZoomed) {
        event.preventDefault();
        uUnzoom();
      }
    });
    uModal.addEventListener('close', function () { uUnzoom(); });
    uModal.querySelector('.u-close').addEventListener('click', function () { uModal.close(); });
    return uModal;
  }

  function uOpen(options) {
    uHeaders = options.headers;
    uOnSelect = options.onSelect;
    if (uController) uController.abort();
    clearTimeout(uSearchTimer);
    ++uSeq;
    uNext = null;
    uBusy(false);
    uUnzoom();
    var modal = unsplashModal();
    if (!modal.open) {
      modal.showModal();
      modal.querySelector('input').focus();
    }
    modal.querySelector('input').value = '';
    if (!uHeaders) {
      // The option is always offered; without a key the picker says what is
      // missing instead of pretending to search.
      uSay('Turn Unsplash on under Settings → Integrations → Unsplash to search and pick free photos.');
      return;
    }
    uLoad('https://api.unsplash.com/photos?per_page=30', true);
  }

  function uGrid() { return unsplashModal().querySelector('.u-grid'); }

  function uBusy(on) {
    uLoading = on;
    var spinner = unsplashModal().querySelector('.u-loading');
    if (spinner) { spinner.hidden = !on; }
  }

  // One line of prose where the grid goes, for waiting, empties and errors.
  function uSay(text) {
    uGrid().textContent = '';
    var row = document.createElement('div');
    row.className = 'u-empty';
    row.textContent = text;
    uGrid().appendChild(row);
  }

  function uLoad(url, fresh) {
    var mine = ++uSeq;
    if (fresh) {
      uNext = null;
      uZoomed && uUnzoom();
      uBusy(true);
      uSay(' ');
    } else {
      uBusy(true);
    }
    if (uController) { uController.abort(); }
    uController = new AbortController();

    fetch(url, {headers: uHeaders || {}, signal: uController.signal})
      .then(function (response) {
        if (response.status === 403 && response.headers.get('x-ratelimit-remaining') === '0') {
          throw new Error('Unsplash API rate limit reached, please try again later.');
        }
        if (!response.ok) {
          return response.json().catch(function () { return null; }).then(function (json) {
            var message = json && json.errors && json.errors[0];
            throw new Error(message || ('Error ' + response.status + ': trouble reaching the Unsplash API'));
          });
        }
        // The next page, when there is one, comes straight off the Link
        // header — the API's own pagination, not a page counter.
        var next = null;
        var link = response.headers.get('link') || '';
        link.split(',').forEach(function (part) {
          var match = /<(.*)>; rel="next"/.exec(part.trim());
          if (match) { next = match[1]; }
        });
        uNext = next;
        return response.json();
      })
      .then(function (json) {
        if (mine !== uSeq) { return; }
        var photos = Array.isArray(json) ? json : (json && json.results) || [];
        if (fresh) {
          uGrid().textContent = '';
          if (!photos.length) { uSay('No photos matched. Try another search.'); }
        }
        photos.forEach(function (photo) { uGrid().appendChild(uTile(photo)); });
        if (!fresh && !photos.length) { uNext = null; }
      })
      .catch(function (err) {
        if (err && err.name === 'AbortError') { return; }
        if (mine !== uSeq) { return; }
        if (fresh) { uSay(err.message || 'Unsplash could not be reached.'); }
      })
      .then(function () {
        if (mine === uSeq) { uBusy(false); }
      });
  }

  // One masonry tile: the photograph at its own shape, over the colour the
  // API gives it, with Ghost's hover laid on top.
  function uTile(photo, zoom) {
    var tile = document.createElement('div');
    tile.className = 'u-tile' + (zoom ? ' zoomed' : '');
    tile.style.background = photo.color || 'transparent';

    var img = document.createElement('img');
    img.loading = 'lazy';
    img.alt = photo.alt_description || '';
    img.src = (photo.urls && photo.urls.regular) || '';
    tile.appendChild(img);

    var hover = document.createElement('div');
    hover.className = 'u-hover';
    var likes = document.createElement('a');
    likes.className = 'u-chip';
    likes.target = '_blank';
    likes.rel = 'noopener noreferrer';
    likes.href = (photo.links && uReferral(photo.links.html)) || '#';
    likes.innerHTML = U_HEART + '<span>' + (photo.likes || 0) + '</span>';
    var down = document.createElement('a');
    down.className = 'u-chip';
    down.target = '_blank';
    down.rel = 'noopener noreferrer';
    down.href = (photo.links && uReferral(photo.links.download)) + (photo.links && photo.links.download ? '&force=true' : '#');
    down.setAttribute('aria-label', 'Download');
    down.innerHTML = U_DOWNLOAD;
    var top = document.createElement('div');
    top.className = 'u-top';
    top.append(likes, down);

    var user = photo.user || {};
    var credit = document.createElement('span');
    credit.className = 'u-credit';
    if (user.profile_image && user.profile_image.medium) {
      var avatar = document.createElement('img');
      avatar.className = 'u-avatar';
      avatar.src = user.profile_image.medium;
      avatar.alt = '';
      credit.appendChild(avatar);
    }
    var name = document.createElement('span');
    name.className = 'u-credit-name';
    name.textContent = user.name || 'Unsplash';
    name.title = name.textContent;
    credit.appendChild(name);

    var insert = document.createElement('button');
    insert.className = 'u-insert';
    insert.type = 'button';
    insert.textContent = 'Insert image';
    insert.addEventListener('click', function (event) {
      event.preventDefault();
      event.stopPropagation();
      uInsert(photo);
    });

    var bottom = document.createElement('div');
    bottom.className = 'u-bottom';
    bottom.append(credit, insert);
    hover.append(top, bottom);
    tile.appendChild(hover);

    if (!zoom) {
      tile.addEventListener('click', function () { uZoom(photo); });
    }
    return tile;
  }

  function uZoom(photo) {
    uZoomed = photo;
    var pane = unsplashModal().querySelector('.u-zoom');
    pane.textContent = '';
    pane.appendChild(uTile(photo, true));
    pane.hidden = false;
    uGrid().hidden = true;
    // Clicking the zoomed photograph is how you step back out of it.
    pane.addEventListener('click', uUnzoom);
  }

  function uUnzoom() {
    if (!uZoomed) { return; }
    uZoomed = null;
    var pane = unsplashModal().querySelector('.u-zoom');
    pane.removeEventListener('click', uUnzoom);
    pane.hidden = true;
    pane.textContent = '';
    uGrid().hidden = false;
  }

  // What Insert writes: the same wide URL and the same credit caption
  // Ghost's picker writes, referral links and all.
  function uInsert(photo) {
    var user = photo.user || {};
    var src = ((photo.urls && photo.urls.regular) || '').replace(/&w=1080/, '&w=2000');
    var credit = '<span>Photo by <a href="' + uReferral(user.links && user.links.html || 'https://unsplash.com/') + '">' +
      uEsc(user.name || 'Unsplash') + '</a> / <a href="' + uReferral('https://unsplash.com/') + '">Unsplash</a></span>';
    uOnSelect(src, credit);
    // Unsplash's guidelines ask each use to be reported; Ghost triggers this
    // endpoint on insert and so does this.
    if (photo.links && photo.links.download_location) {
      fetch(photo.links.download_location, {headers: uHeaders || {}}).catch(function () {});
    }
    uModal.close();
  }

  window.GhostUnsplash = {open: uOpen};
})();
