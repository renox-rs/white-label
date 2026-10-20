// How forms that post with htmx show a refusal (the app's layout loads this
// file; a file rather than an inline script, so it runs under CSP=strict).
//
// Renox's own script covers a 422 with field errors (JSON from `Valid`): each
// message goes next to its field. This file covers the rest, which the
// modules answer with `Error::Status` (a business rule: "Emirates ID
// missing; ...") and which would otherwise do nothing visible, because htmx
// does not swap an error answer:
//
//  - an error answer that is a page (the app's errors/default.html carries the
//    message in `data-error-detail`) shows its message as an alert at the top
//    of the modal or form that sent the request, and the modal stays open;
//  - a JSON 422 without field errors does the same with its `message`;
//  - outside a modal or form the message is a toast;
//  - the confirmation modals (`rx-confirm`) post a plain form: they are sent
//    with htmx too, so a refusal shows in the modal and a success closes it.
(function () {
  var FALLBACK = {
    401: "Sign in again to continue.",
    403: "You are not allowed to do this.",
    404: "This is no longer there.",
    409: "This changed meanwhile. Reload the page and try again.",
    419: "This page expired. Reload it and try again.",
    422: "This could not be done.",
  };
  var GENERIC = "Something went wrong. Try again.";
  var ICON =
    '<svg viewBox="0 0 20 20" width="20" height="20"><circle cx="10" cy="10" r="9" fill="currentColor"/>' +
    '<path d="M7 7l6 6M13 7l-6 6" stroke="#fff" stroke-width="2" stroke-linecap="round"/></svg>';

  function messageOf(xhr) {
    var type = xhr.getResponseHeader("Content-Type") || "";
    var text = xhr.responseText || "";
    if (type.indexOf("application/json") === 0) {
      try {
        var body = JSON.parse(text);
        return { message: body.message || "", fields: Object.keys(body.errors || {}).length };
      } catch (_) {
        return { message: "", fields: 0 };
      }
    }
    if (type.indexOf("text/html") === 0 && text) {
      var doc = new DOMParser().parseFromString(text, "text/html");
      var marker = doc.querySelector("[data-error-detail]");
      if (marker) return { message: marker.getAttribute("data-error-detail") || "", fields: 0 };
    }
    return { message: "", fields: 0 };
  }

  function container(elt) {
    var dialog = elt && elt.closest && elt.closest("dialog");
    if (dialog) return dialog.querySelector(".rx-sheet__inner") || dialog;
    return elt && elt.closest && elt.closest("form");
  }

  function clear(root) {
    if (!root || !root.querySelectorAll) return;
    root.querySelectorAll("[data-refusal]").forEach(function (el) { el.remove(); });
  }

  function show(elt, message) {
    var root = container(elt);
    if (!root) {
      document.dispatchEvent(new CustomEvent("renox:toast", { detail: { kind: "error", message: message } }));
      return;
    }
    clear(root);
    var alert = document.createElement("div");
    alert.className = "rx-alert rx-alert--error";
    alert.setAttribute("role", "alert");
    alert.setAttribute("tabindex", "-1");
    alert.setAttribute("data-refusal", "");
    var icon = document.createElement("span");
    icon.className = "rx-alert__icon";
    icon.setAttribute("aria-hidden", "true");
    icon.innerHTML = ICON;
    var body = document.createElement("div");
    var text = document.createElement("p");
    text.className = "rx-alert__message";
    text.textContent = message;
    text.style.whiteSpace = "pre-line";
    body.appendChild(text);
    alert.appendChild(icon);
    alert.appendChild(body);
    // Above the title of a modal, above the first field of a form.
    root.insertBefore(alert, root.firstChild);
    if (alert.focus) alert.focus({ preventScroll: false });
  }

  document.addEventListener("htmx:beforeRequest", function (event) {
    clear(container(event.detail.elt));
  });

  document.addEventListener("htmx:beforeSwap", function (event) {
    var xhr = event.detail.xhr;
    if (!xhr || xhr.status < 400) return;
    var found = messageOf(xhr);
    // Field errors are Renox's: next to their fields.
    if (xhr.status === 422 && found.fields > 0) return;
    event.detail.shouldSwap = false;
    show(event.detail.requestConfig && event.detail.requestConfig.elt,
      found.message || FALLBACK[xhr.status] || GENERIC);
  });

  // The request never got an answer (the network, a stopped server).
  document.addEventListener("htmx:sendError", function (event) {
    show(event.detail.elt, GENERIC);
  });

  // The confirmation modals: a plain form in the kit, sent with htmx here.
  function upgrade(root) {
    if (!window.htmx || !root || !root.querySelectorAll) return;
    var forms = root.matches && root.matches('dialog[data-rx-component="confirm"] form')
      ? [root]
      : root.querySelectorAll('dialog[data-rx-component="confirm"] form');
    forms.forEach(function (form) {
      if (form.hasAttribute("hx-post")) return;
      form.setAttribute("hx-post", form.getAttribute("action"));
      form.setAttribute("hx-swap", "none");
      // The kit closes the modal on a success and clears it.
      form.setAttribute("data-rx-action", "");
      window.htmx.process(form);
    });
  }
  function upgradeAll() { upgrade(document.body); }
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", upgradeAll);
  else upgradeAll();
  document.addEventListener("htmx:load", function (event) { upgrade(event.target); });

  // An answer that was a redirect (a handler written for a plain form post):
  // htmx followed it quietly, so go there, as the browser would have.
  document.addEventListener("htmx:afterRequest", function (event) {
    var form = event.detail.elt;
    if (!event.detail.successful || !form || !form.matches) return;
    if (!form.matches('dialog[data-rx-component="confirm"] form')) return;
    var xhr = event.detail.xhr;
    if (xhr.getResponseHeader("HX-Redirect") || xhr.getResponseHeader("HX-Refresh")) return;
    var target = xhr.responseURL;
    if (target && target !== new URL(form.getAttribute("hx-post"), location.href).href) {
      window.location.href = target;
    }
  });
})();
