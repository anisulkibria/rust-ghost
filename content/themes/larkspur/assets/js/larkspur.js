(function () {
    const menu = document.querySelector('.menu-toggle');
    const nav = document.getElementById('site-nav');
    if (menu && nav) {
        menu.hidden = false;
        const setOpen = function (open) {
            nav.classList.toggle('is-open', open);
            menu.setAttribute('aria-expanded', String(open));
        };
        menu.addEventListener('click', function () {
            const open = menu.getAttribute('aria-expanded') !== 'true';
            setOpen(open);
            if (open) nav.querySelector('a[href]')?.focus();
        });
        document.addEventListener('keydown', function (event) {
            if (event.key === 'Escape' && nav.classList.contains('is-open')) {
                setOpen(false);
                menu.focus();
            }
        });
        document.documentElement.classList.add('has-js');
    }
})();
