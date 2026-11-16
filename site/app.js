(() => {
  const copyText = async (text, codeEl) => {
    if (navigator.clipboard && typeof navigator.clipboard.writeText === "function") {
      try {
        await Promise.race([
          navigator.clipboard.writeText(text),
          new Promise((_, reject) => window.setTimeout(() => reject(new Error("timeout")), 300)),
        ]);
        return true;
      } catch {
        // Fallback below
      }
    }
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.setAttribute("readonly", "");
    ta.style.position = "fixed";
    ta.style.opacity = "0";
    document.body.appendChild(ta);
    ta.select();
    let ok = false;
    try {
      ok = document.execCommand("copy");
    } catch {
      ok = false;
    }
    document.body.removeChild(ta);
    if (!ok && codeEl) {
      const sel = window.getSelection();
      const range = document.createRange();
      range.selectNodeContents(codeEl);
      sel?.removeAllRanges();
      sel?.addRange(range);
    }
    return ok;
  };

  for (const btn of document.querySelectorAll("[data-copy-target]")) {
    btn.addEventListener("click", async () => {
      const id = btn.getAttribute("data-copy-target");
      const codeEl = id ? document.getElementById(id) : null;
      if (!codeEl) return;
      const ok = await copyText(codeEl.textContent || "", codeEl);
      const prev = btn.textContent;
      btn.setAttribute("aria-live", "polite");
      btn.textContent = ok ? "Copied" : "Select and copy";
      if (ok) btn.setAttribute("data-copied", "true");
      window.setTimeout(() => {
        btn.textContent = prev;
        btn.removeAttribute("data-copied");
      }, 1800);
    });
  }

  // Screenshot lightbox using native <dialog>
  const dialog = document.getElementById("lightbox");
  const lbImg = document.getElementById("lightbox-img");
  const lbCaption = document.getElementById("lightbox-caption");
  const lbClose = document.getElementById("lightbox-close");
  let lastTrigger = null;

  if (dialog && typeof dialog.showModal === "function" && lbImg && lbCaption && lbClose) {
    for (const trigger of document.querySelectorAll(".shot-frame")) {
      trigger.addEventListener("click", () => {
        const img = trigger.querySelector("img");
        if (!img) return;
        lastTrigger = trigger;
        lbImg.src = img.currentSrc || img.src;
        lbImg.alt = img.alt;
        const fig = trigger.closest("figure");
        const cap = fig ? fig.querySelector("figcaption") : null;
        lbCaption.textContent = cap ? cap.textContent : img.alt;
        dialog.showModal();
        lbClose.focus();
      });
    }

    const closeDialog = () => {
      dialog.close();
      lbImg.removeAttribute("src");
      if (lastTrigger && typeof lastTrigger.focus === "function") {
        lastTrigger.focus();
      }
      lastTrigger = null;
    };

    lbClose.addEventListener("click", closeDialog);
    dialog.addEventListener("click", (e) => {
      if (e.target === dialog) closeDialog();
    });
    dialog.addEventListener("cancel", (e) => {
      e.preventDefault();
      closeDialog();
    });
  }
})();
