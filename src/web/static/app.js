(() => {
  const $ = (selector, root = document) => root.querySelector(selector);
  const $$ = (selector, root = document) => [...root.querySelectorAll(selector)];
  const shade = $("#shade"), sheet = $("#sheet"), sheetIn = $("#sheet-in");
  const toastEl = $("#toast"), toastText = $("#toast-t"), undoButton = $("#toast-undo");
  let toastTimer, undoAction = null, selectedSentence = null, lastFocus = null;

  function showToast(message, undo = null) {
    if (!toastEl) return;
    toastText.textContent = message;
    undoAction = undo;
    undoButton.hidden = !undo;
    toastEl.classList.add("on");
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => toastEl.classList.remove("on"), 3500);
  }

  function openSheet(content, focusTarget = null) {
    if (!sheet || !shade) return;
    if (!sheet.classList.contains("on")) lastFocus = document.activeElement;
    sheetIn.replaceChildren(content);
    sheet.classList.add("on");
    shade.classList.add("on");
    sheet.setAttribute("aria-hidden", "false");
    (focusTarget || $("a,button", sheetIn))?.focus();
  }

  function closeSheet() {
    if (!sheet || !shade) return;
    if (!sheet.classList.contains("on")) return;
    sheet.classList.remove("on");
    shade.classList.remove("on");
    sheet.setAttribute("aria-hidden", "true");
    selectedSentence?.classList.remove("sel");
    selectedSentence = null;
    sheetIn.replaceChildren();
    lastFocus?.focus?.();
    lastFocus = null;
  }

  function element(tag, className, text) {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
  }

  function sentenceSheet(link) {
    selectedSentence?.classList.remove("sel");
    selectedSentence = link;
    link.classList.add("sel");
    const content = document.createDocumentFragment();
    content.append(element("div", "src", link.dataset.outlet || "Source"));
    if (link.dataset.original) {
      content.append(element("p", "orig", `“${link.dataset.original}”`));
    }
    if (link.dataset.verified === "false") {
      content.append(element("p", "warn", "This line could not be matched to the article. Check the source before relying on it."));
    }
    const open = element("a", "act primary", `Open in ${link.dataset.outlet || "source"}`);
    open.href = link.dataset.articleUrl || link.href;
    open.target = "_blank";
    open.rel = "noopener";
    content.append(open);
    const close = element("button", "act plain", "Close");
    close.type = "button";
    close.dataset.closeSheet = "true";
    content.append(close);
    openSheet(content, open);
  }

  function share(link) {
    const url = new URL(link.getAttribute("href"), location.href).href;
    const title = link.dataset.shareTitle || document.title;
    if (navigator.share) {
      navigator.share({ title, url }).catch(() => {});
      return;
    }
    const copy = navigator.clipboard?.writeText(url) || Promise.reject();
    copy.then(() => showToast("Link copied"), () => {
      const field = element("textarea");
      field.value = url;
      field.style.position = "fixed";
      field.style.opacity = "0";
      document.body.append(field);
      field.select();
      const copied = document.execCommand("copy");
      field.remove();
      showToast(copied ? "Link copied" : "Copy this link from the address bar");
    });
  }

  async function postForm(form, values) {
    const body = new URLSearchParams(values);
    const response = await fetch(form.action, {
      method: "POST",
      headers: { "Content-Type": "application/x-www-form-urlencoded", "Accept": "application/json", "X-Requested-With": "fetch" },
      body,
    });
    if (!response.ok) throw new Error("Request failed");
  }

  function updateVotes(value) {
    $$('[data-action="vote"]').forEach(form => {
      const button = $("button", form), field = $("[name=v]", form), direction = Number(form.dataset.direction || field.value);
      const active = value === direction && value !== 0;
      field.value = active ? "0" : String(direction);
      form.dataset.currentVote = String(value);
      button.classList.toggle("on", active);
      button.setAttribute("aria-pressed", String(active));
    });
  }

  function updateSave(value) {
    const form = $('[data-action="save"]');
    if (!form) return;
    const button = $("button", form), field = $("[name=saved]", form);
    field.value = String(!value);
    button.classList.toggle("on", value);
    button.setAttribute("aria-pressed", String(value));
    $(".ic span:last-child", button).textContent = value ? "Saved" : "Save";
  }

  function setActionBusy(action, busy) {
    $$(`[data-action="${action}"] button`).forEach(button => { button.disabled = busy; });
  }

  function applyAction(form, isVote, value) {
    if (isVote) updateVotes(value); else updateSave(value);
    const field = $(isVote ? "[name=v]" : "[name=saved]", form);
    if (field) field.value = isVote ? (value === Number(form.dataset.direction) ? "0" : form.dataset.direction) : String(!value);
  }

  async function persistAction(form, isVote, value) {
    await postForm(form, isVote ? { v: String(value) } : { saved: String(value) });
  }

  document.addEventListener("click", event => {
    const close = event.target.closest("[data-close-sheet]");
    if (close) { event.preventDefault(); closeSheet(); return; }
    const sentence = event.target.closest(".body a.s");
    if (sentence) { event.preventDefault(); sentenceSheet(sentence); return; }
    const more = event.target.closest("#more > summary");
    if (more) {
      event.preventDefault();
      const menu = $(".more-menu", more.parentElement);
      if (menu) {
        const content = document.createDocumentFragment();
        content.append(menu.cloneNode(true));
        const close = element("button", "act plain", "Close");
        close.type = "button";
        close.dataset.closeSheet = "true";
        content.append(close);
        openSheet(content);
      }
      return;
    }
    const shareLink = event.target.closest(".share-link");
    if (shareLink) { event.preventDefault(); share(shareLink); return; }
    if (event.target === shade) closeSheet();
  });

  document.addEventListener("submit", async event => {
    const form = event.target.closest('form[data-action="vote"],form[data-action="save"]');
    if (!form) return;
    event.preventDefault();
    const isVote = form.dataset.action === "vote";
    const field = $(isVote ? "[name=v]" : "[name=saved]", form);
    const next = isVote ? Number(field.value) : field.value === "true";
    const prior = isVote ? Number(form.dataset.currentVote || "0") : !next;
    const action = isVote ? "vote" : "save";
    if ($(`[data-action="${action}"] button:disabled`)) return;
    undoAction = null;
    undoButton.hidden = true;
    setActionBusy(action, true);
    applyAction(form, isVote, next);
    try {
      await persistAction(form, isVote, next);
      showToast(isVote ? (next === 1 ? "Marked relevant" : next === -1 ? "Marked not for me" : "Vote removed") : (next ? "Saved" : "Removed from saved"), async () => {
        setActionBusy(action, true);
        try {
          await persistAction(form, isVote, prior);
          applyAction(form, isVote, prior);
          showToast(isVote ? "Vote undone" : "Change undone");
        } catch {
          showToast("Could not undo that action", async () => {
            try { await persistAction(form, isVote, prior); applyAction(form, isVote, prior); showToast("Change undone"); }
            catch { showToast("Could not undo that action"); }
          });
        } finally { setActionBusy(action, false); }
      });
    } catch {
      applyAction(form, isVote, prior);
      try { await persistAction(form, isVote, prior); showToast("Could not save that change; previous state restored"); }
      catch { showToast("Could not save or restore that change", async () => {
        try { await persistAction(form, isVote, prior); showToast("Previous state restored"); }
        catch { showToast("Could not restore previous state"); }
      }); }
    } finally { setActionBusy(action, false); }
  });

  undoButton?.addEventListener("click", async () => {
    const undo = undoAction;
    undoAction = null;
    toastEl.classList.remove("on");
    await undo?.();
  });
  shade?.addEventListener("click", closeSheet);
  document.addEventListener("keydown", event => {
    if (event.key === "Escape") closeSheet();
    if (event.key === "Tab" && sheet?.classList.contains("on")) {
      const focusable = $$('a[href],button:not([disabled]),input:not([disabled]),select:not([disabled]),textarea:not([disabled]),[tabindex]:not([tabindex="-1"])', sheet)
        .filter(node => !node.hidden && node.getAttribute("aria-hidden") !== "true");
      if (!focusable.length) { event.preventDefault(); sheet.focus(); return; }
      const first = focusable[0], last = focusable[focusable.length - 1];
      if (event.shiftKey && (document.activeElement === first || !sheet.contains(document.activeElement))) { event.preventDefault(); last.focus(); }
      else if (!event.shiftKey && (document.activeElement === last || !sheet.contains(document.activeElement))) { event.preventDefault(); first.focus(); }
    }
  });
})();
