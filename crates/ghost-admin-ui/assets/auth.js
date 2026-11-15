// The signed-out screens' one piece of behaviour: reveal a typed password.
//
// Sign in, setup, invite redemption and the password reset all render a
// password field through `auth::password_field`, and each one carries the
// button this drives. There is nothing else on these pages that needs a
// script, and nothing here talks to the server — the form posts the same
// fields whether or not this ran.
//
// The button is hidden by CSS until the head script removes `no-js` from
// `<html>`, so a browser that cannot run this is never offered a control
// that would do nothing.
(function () {
  'use strict';

  // `aria-pressed` is the state, not a class of our own: the button already
  // has to carry it for a screen reader, and keeping one source of truth
  // means the CSS, the label and the assistive announcement cannot disagree.
  function apply(button, input, shown) {
    button.setAttribute('aria-pressed', shown ? 'true' : 'false');
    input.type = shown ? 'text' : 'password';
    var label = shown ? 'Hide password' : 'Show password';
    button.setAttribute('aria-label', label);
    button.setAttribute('title', label);
  }

  document.querySelectorAll('.auth-reveal').forEach(function (button) {
    var field = button.closest('.auth-pw');
    var input = field && field.querySelector('input');
    if (!input) {
      return;
    }
    button.addEventListener('click', function () {
      var shown = button.getAttribute('aria-pressed') === 'true';
      apply(button, input, !shown);
      // Focus goes back to the field, at the end of what is already typed:
      // the reveal is something you do *while* typing, and leaving the caret
      // behind on the button means the next keystroke is lost.
      input.focus();
      var end = input.value.length;
      if (input.setSelectionRange) {
        try {
          input.setSelectionRange(end, end);
        } catch (e) {
          // `setSelectionRange` throws on some input types in older engines.
          // The reveal still worked; the caret position is a nicety.
        }
      }
    });
  });

  // A revealed password must not survive the page. Submitting re-hides it so
  // a browser restoring this form from the back/forward cache — which keeps
  // field values and the DOM as they were — does not redisplay somebody's
  // password on a screen they have walked away from.
  document.querySelectorAll('form').forEach(function (form) {
    form.addEventListener('submit', function () {
      form.querySelectorAll('.auth-pw input[type="text"]').forEach(function (input) {
        var field = input.closest('.auth-pw');
        var button = field && field.querySelector('.auth-reveal');
        if (button) {
          apply(button, input, false);
        }
      });
    });
  });
})();
