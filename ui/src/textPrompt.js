import { escapeHtml } from "./utils/ircColors.js";

// A one-line question in the app's own dialog. The script menus ask their $$? prompts through it,
// because window.prompt does not reliably answer in the desktop window. Resolves with the text,
// or with null when the question is cancelled.
export function askText(question) {
  return new Promise((resolve) => {
    const backdrop = document.createElement("div");
    backdrop.className = "modal-overlay text-prompt-backdrop";
    backdrop.innerHTML = `
      <form class="modal-card text-prompt" autocomplete="off">
        <div class="text-prompt-question">${escapeHtml(question)}</div>
        <input class="form-input text-prompt-input" type="text" />
        <div class="text-prompt-buttons">
          <button type="button" class="btn" data-cancel>Cancel</button>
          <button type="submit" class="btn btn-primary">OK</button>
        </div>
      </form>`;
    const form = backdrop.querySelector("form");
    const input = backdrop.querySelector("input");
    const finish = (value) => {
      backdrop.remove();
      resolve(value);
    };
    form.addEventListener("submit", (e) => {
      e.preventDefault();
      finish(input.value);
    });
    backdrop.querySelector("[data-cancel]").addEventListener("click", () => finish(null));
    backdrop.addEventListener("keydown", (e) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        finish(null);
      }
    });
    document.body.appendChild(backdrop);
    input.focus();
  });
}
