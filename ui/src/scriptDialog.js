import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { escapeHtml, parseIrcToHtml } from "./utils/ircColors.js";
import { askText } from "./textPrompt.js";

const BACKDROP_ID = "script-dialog-backdrop";
let activeDialog = null; // { id, title, spec, drafts: {}, activeTab: string, network: string, channel: string, lastSig: string }
let listenersAttached = false;
let pressStartedOnBackdrop = false;
let isTauri = typeof window !== "undefined" && Boolean(window.__TAURI_INTERNALS__);
let dialogOptions = {};

export function isScriptDialogOpen() {
  return activeDialog !== null;
}

export function getOpenScriptDialogId() {
  return activeDialog ? activeDialog.id : null;
}

export function getOpenScriptDialogTarget() {
  if (!activeDialog || !activeDialog.spec || !activeDialog.spec.auto_refresh) {
    return null;
  }
  return {
    id: activeDialog.id,
    network: activeDialog.network || "",
    channel: activeDialog.channel || "",
  };
}

export function closeOpenScriptDialog(notifyBackend = true) {
  if (!activeDialog) return;
  const current = activeDialog;
  activeDialog = null;

  const container = document.getElementById("modal-container");
  if (container) {
    const backdrop = document.getElementById(BACKDROP_ID);
    if (backdrop) backdrop.remove();
  }

  if (notifyBackend && isTauri) {
    invoke("script_dialog_event", {
      dialogId: current.id,
      event: { type: "close" },
      network: current.network || null,
      channel: current.channel || null,
    }).catch((err) => console.error("script_dialog_event close error:", err));
  }
}

export function refreshOpenScriptDialog(channelSignature = null) {
  if (!activeDialog || !activeDialog.spec || !activeDialog.spec.auto_refresh) return;
  if (!isTauri) return;

  if (!document.getElementById(BACKDROP_ID)) {
    activeDialog = null;
    return;
  }

  // Only refresh when signature actually changed
  if (channelSignature && channelSignature === activeDialog.lastSig) return;
  if (channelSignature) activeDialog.lastSig = channelSignature;

  invoke("script_dialog_event", {
    dialogId: activeDialog.id,
    event: { type: "refresh", tab: activeDialog.activeTab },
    network: activeDialog.network || null,
    channel: activeDialog.channel || null,
  }).catch((err) => console.error("script_dialog_event refresh error:", err));
}

export function initScriptDialog(options = {}) {
  dialogOptions = options || {};
  if (!isTauri) return;
  if (listenersAttached) return;
  listenersAttached = true;

  listen("script-dialog", (event) => {
    const payload = event.payload;
    if (!payload || !payload.op || !payload.id) return;

    if (payload.op === "open") {
      handleDialogOpen(payload.id, payload.title || "Dialog", payload.spec || {});
    } else if (payload.op === "update") {
      handleDialogUpdate(payload.id, payload.title, payload.spec);
    } else if (payload.op === "close") {
      if (activeDialog && activeDialog.id.toLowerCase() === payload.id.toLowerCase()) {
        closeOpenScriptDialog(false);
      }
    }
  });

  window.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && activeDialog) {
      e.stopPropagation();
      closeOpenScriptDialog(true);
    }
  });
}

function handleDialogOpen(id, title, spec) {
  if (typeof dialogOptions.onBeforeOpen === "function") {
    try {
      dialogOptions.onBeforeOpen();
    } catch (err) {
      console.error("onBeforeOpen error:", err);
    }
  }

  // Save or reset state
  activeDialog = {
    id,
    title,
    spec,
    drafts: {},
    activeTab: spec.active_tab || (spec.tabs && spec.tabs[0] ? spec.tabs[0].id : ""),
    network: spec.network || "",
    channel: spec.channel || "",
    lastSig: "",
  };

  renderScriptDialog();
}

function handleDialogUpdate(id, newTitle, newSpec) {
  if (!activeDialog || activeDialog.id.toLowerCase() !== id.toLowerCase()) return;

  if (newTitle !== undefined && newTitle !== null) {
    activeDialog.title = newTitle;
  }
  if (newSpec !== undefined && newSpec !== null) {
    // Preserve activeTab unless explicitly specified in newSpec
    const prevTab = activeDialog.activeTab;
    activeDialog.spec = newSpec;
    if (newSpec.active_tab) {
      activeDialog.activeTab = newSpec.active_tab;
    } else if (prevTab && newSpec.tabs && newSpec.tabs.some((t) => t.id === prevTab)) {
      activeDialog.activeTab = prevTab;
    }
    if (newSpec.network) activeDialog.network = newSpec.network;
    if (newSpec.channel) activeDialog.channel = newSpec.channel;
  }

  renderScriptDialog();
}

function dispatchEvent(eventData) {
  if (!activeDialog || !isTauri) return;

  // Collect form drafts
  collectDraftsFromDOM();

  const payload = {
    ...eventData,
    tab: activeDialog.activeTab,
    drafts: { ...activeDialog.drafts },
  };

  invoke("script_dialog_event", {
    dialogId: activeDialog.id,
    event: payload,
    network: activeDialog.network || null,
    channel: activeDialog.channel || null,
  }).catch((err) => console.error("script_dialog_event error:", err));
}

function collectDraftsFromDOM() {
  if (!activeDialog) return;
  const modal = document.querySelector(".script-dialog");
  if (!modal) return;

  // Inputs
  modal.querySelectorAll("input[data-id], textarea[data-id], select[data-id]").forEach((el) => {
    const id = el.dataset.id;
    if (el.type === "checkbox") {
      activeDialog.drafts[id] = el.checked;
    } else {
      activeDialog.drafts[id] = el.value;
    }
  });
}

function renderScriptDialog() {
  if (!activeDialog) return;
  const container = document.getElementById("modal-container");
  if (!container) return;

  // Save active focus and scroll positions before render
  const activeEl = document.activeElement;
  const savedFocusId = activeEl?.dataset?.id || activeEl?.id || null;
  const savedSelStart = typeof activeEl?.selectionStart === "number" ? activeEl.selectionStart : null;
  const savedSelEnd = typeof activeEl?.selectionEnd === "number" ? activeEl.selectionEnd : null;
  const bodyEl = container.querySelector(".script-dlg-body");
  const savedScrollTop = bodyEl ? bodyEl.scrollTop : 0;

  const spec = activeDialog.spec || {};
  const tabs = spec.tabs || [];
  const controls = spec.controls || [];
  const tabContents = spec.tab_contents || {};
  const buttons = spec.buttons || [
    { id: "ok", label: "OK", primary: true },
    { id: "close", label: "Close" },
  ];

  let currentTabControls = [];
  if (tabs.length > 0) {
    currentTabControls = tabContents[activeDialog.activeTab] || controls.filter((c) => !c.tab || c.tab === activeDialog.activeTab);
  } else {
    currentTabControls = controls;
  }

  const tabsHtml = tabs.length > 0 ? `
    <div class="script-dlg-tab-bar" role="tablist">
      ${tabs.map((tab) => `
        <button type="button" class="script-dlg-tab-btn script-dlg-tab ${tab.id === activeDialog.activeTab ? "is-active" : ""}" data-tab="${escapeHtml(tab.id)}" role="tab" aria-selected="${tab.id === activeDialog.activeTab}">
          ${escapeHtml(tab.label)}
        </button>
      `).join("")}
    </div>
  ` : "";

  const controlsHtml = currentTabControls.map((ctrl) => renderControl(ctrl)).join("");

  const buttonsHtml = `
    <div class="script-dlg-footer">
      ${buttons.map((b) => `
        <button type="button" class="btn ${b.primary ? "btn-primary" : "btn-secondary"} script-dlg-btn" data-btn-id="${escapeHtml(b.id)}">
          ${escapeHtml(b.label)}
        </button>
      `).join("")}
    </div>
  `;

  let noticeHtml = "";
  if (spec.notice) {
    const lvl = spec.notice.level || "info";
    noticeHtml = `<div class="script-dlg-notice is-${escapeHtml(lvl)}">${escapeHtml(spec.notice.text)}</div>`;
  }

  container.innerHTML = `
    <div id="${BACKDROP_ID}" class="modal-overlay">
      <div class="modal-card script-dialog" role="dialog" aria-modal="true" aria-labelledby="script-dlg-title">
        <div class="script-dlg-header">
          <div id="script-dlg-title" class="script-dlg-title">${escapeHtml(activeDialog.title)}</div>
          <button type="button" class="btn-icon-sm script-dlg-close-x" title="Close" aria-label="Close">✕</button>
        </div>
        ${noticeHtml}
        ${tabsHtml}
        <div class="script-dlg-body">
          ${controlsHtml}
        </div>
        ${buttonsHtml}
      </div>
    </div>
  `;

  bindDomEvents(container);

  // Restore scroll and focus
  const newBody = container.querySelector(".script-dlg-body");
  if (newBody && savedScrollTop) {
    newBody.scrollTop = savedScrollTop;
  }
  if (savedFocusId) {
    const escaped = CSS.escape(savedFocusId);
    const newFocus = container.querySelector(`[data-id="${escaped}"]`) || container.querySelector(`#${escaped}`);
    if (newFocus) {
      newFocus.focus();
      if (savedSelStart !== null && typeof newFocus.setSelectionRange === "function") {
        try {
          newFocus.setSelectionRange(savedSelStart, savedSelEnd ?? savedSelStart);
        } catch { }
      }
    }
  }
}

function renderControl(ctrl) {
  if (!ctrl || !ctrl.type) return "";

  switch (ctrl.type) {
    case "notice":
      return `
        <div class="script-dlg-notice is-${escapeHtml(ctrl.level || "info")}">
          ${escapeHtml(ctrl.text || "")}
        </div>
      `;

    case "info_box": {
      const items = ctrl.items || [];
      return `
        <div class="script-dlg-box">
          ${ctrl.title ? `<div class="script-dlg-box-title">${escapeHtml(ctrl.title)}</div>` : ""}
          <div class="script-dlg-info-grid">
            ${items.map((it) => `
              <div class="script-dlg-info-label">${escapeHtml(it.label)}:</div>
              <div class="script-dlg-info-val">${escapeHtml(String(it.value ?? ""))}</div>
            `).join("")}
          </div>
        </div>
      `;
    }

    case "checkbox_grid": {
      const items = ctrl.items || [];
      const cols = ctrl.columns || 2;
      return `
        <div class="script-dlg-box">
          ${ctrl.title ? `<div class="script-dlg-box-title">${escapeHtml(ctrl.title)}</div>` : ""}
          <div class="script-dlg-check-grid" style="grid-template-columns: repeat(${cols}, minmax(0, 1fr));">
            ${items.map((it) => {
              const checked = activeDialog.drafts[it.id] !== undefined ? activeDialog.drafts[it.id] : it.checked;
              return `
                <label class="script-dlg-check-item ${it.disabled ? "is-disabled" : ""}">
                  <input type="checkbox" data-id="${escapeHtml(it.id)}" ${checked ? "checked" : ""} ${it.disabled ? "disabled" : ""} />
                  <span>${escapeHtml(it.label)}</span>
                </label>
              `;
            }).join("")}
          </div>
        </div>
      `;
    }

    case "mode_inputs": {
      const items = ctrl.items || [];
      return `
        <div class="script-dlg-box script-dlg-mode-box">
          <div class="script-dlg-mode-inputs">
            ${items.map((it) => {
              const enabled = activeDialog.drafts[it.id + "_enable"] !== undefined ? activeDialog.drafts[it.id + "_enable"] : it.enabled;
              const val = activeDialog.drafts[it.id] !== undefined ? activeDialog.drafts[it.id] : (it.value ?? "");
              const isDisabled = !enabled || it.disabled;
              return `
                <div class="script-dlg-mode-row">
                  <label class="script-dlg-check-item ${it.disabled ? "is-disabled" : ""}">
                    <input type="checkbox" data-id="${escapeHtml(it.id)}_enable" ${enabled ? "checked" : ""} ${it.disabled ? "disabled" : ""} />
                    <span>${escapeHtml(it.label)}</span>
                  </label>
                  <input class="form-input script-dlg-input" data-id="${escapeHtml(it.id)}" type="${it.is_password ? "password" : "text"}" value="${escapeHtml(String(val))}" ${isDisabled ? "disabled" : ""} spellcheck="false" placeholder="${escapeHtml(it.placeholder || "")}" />
                </div>
              `;
            }).join("")}
          </div>
        </div>
      `;
    }

    case "topic_editor": {
      const curTopic = ctrl.current ? parseIrcToHtml(ctrl.current) : "<em>No topic set.</em>";
      const curBy = ctrl.current_by ? `Set by ${escapeHtml(ctrl.current_by)}` : "";
      const curAt = ctrl.current_at ? ` at ${escapeHtml(ctrl.current_at)}` : "";
      const maxLen = ctrl.max_len || 500;
      const draftVal = activeDialog.drafts[ctrl.id || "topic"] !== undefined ? activeDialog.drafts[ctrl.id || "topic"] : (ctrl.draft ?? ctrl.current ?? "");
      const remaining = Math.max(0, maxLen - draftVal.length);
      const pct = Math.min(100, Math.round((draftVal.length / maxLen) * 100));
      const history = ctrl.history || [];

      return `
        <div class="script-dlg-topic-section">
          <div class="script-dlg-box">
            <div class="script-dlg-box-title">Current Topic</div>
            <div class="script-dlg-current-topic">${curTopic}</div>
            ${curBy || curAt ? `<div class="script-dlg-topic-meta">${curBy}${curAt}</div>` : ""}
          </div>

          <div class="script-dlg-box">
            <div class="script-dlg-box-title">New Topic</div>
            <div class="script-dlg-topic-input-wrap">
              <textarea id="script-dlg-topic-input" class="form-input script-dlg-textarea" data-id="${escapeHtml(ctrl.id || "topic")}" rows="2" maxlength="${maxLen}" spellcheck="false">${escapeHtml(draftVal)}</textarea>
            </div>

            <div class="script-dlg-topic-bar-row">
              <div class="script-dlg-progress-track" title="${draftVal.length} / ${maxLen} characters">
                <div class="script-dlg-progress-bar ${pct > 90 ? "is-warning" : ""}" style="width: ${pct}%;"></div>
              </div>
              <div class="script-dlg-chars-left">${remaining} remaining</div>
            </div>

            <div class="script-dlg-box-title" style="margin-top: 8px;">Live Preview</div>
            <div class="script-dlg-current-topic script-dlg-topic-preview" id="script-dlg-topic-preview">
              ${draftVal ? parseIrcToHtml(draftVal) : "<em>Empty</em>"}
            </div>

            ${history.length > 0 ? `
              <div class="script-dlg-history-row">
                <div class="script-dlg-box-title">Topic History</div>
                <select class="form-input script-dlg-history-select" id="script-dlg-topic-history-select">
                  <option value="">-- Choose previous topic --</option>
                  ${history.map((h, idx) => `
                    <option value="${idx}">${escapeHtml(h.topic?.slice(0, 60) || "")} (${escapeHtml(h.by || "unknown")})</option>
                  `).join("")}
                </select>
              </div>
            ` : ""}

            <div class="script-dlg-topic-actions">
              <button type="button" class="btn btn-secondary script-dlg-action-btn" data-action="set_topic" data-control="${escapeHtml(ctrl.id || "topic")}">Apply Topic</button>
              <button type="button" class="btn btn-secondary script-dlg-action-btn" data-action="clear_topic" data-control="${escapeHtml(ctrl.id || "topic")}">Clear</button>
            </div>
          </div>
        </div>
      `;
    }

    case "table": {
      const cols = ctrl.columns || [];
      const rows = ctrl.rows || [];
      const actions = ctrl.actions || [];
      const selectedSet = new Set(activeDialog.drafts[ctrl.id + "_selected"] || []);

      return `
        <div class="script-dlg-table-container">
          <div class="script-dlg-table-scroll">
            <table class="script-dlg-table" data-table-id="${escapeHtml(ctrl.id)}">
              <thead>
                <tr>
                  <th class="th-chk"><input type="checkbox" class="script-dlg-select-all" data-table="${escapeHtml(ctrl.id)}" /></th>
                  ${cols.map((col) => `
                    <th class="script-dlg-sortable-th" data-col="${escapeHtml(col.id)}" data-table="${escapeHtml(ctrl.id)}">
                      ${escapeHtml(col.label)}
                      ${ctrl.sort_by === col.id ? (ctrl.sort_desc ? " ▼" : " ▲") : ""}
                    </th>
                  `).join("")}
                </tr>
              </thead>
              <tbody>
                ${rows.length === 0 ? `
                  <tr><td colspan="${cols.length + 1}" class="script-dlg-empty-td">${escapeHtml(ctrl.empty_text || "No entries listed.")}</td></tr>
                ` : rows.map((row) => {
                  const isChecked = selectedSet.has(row.id || row.mask);
                  return `
                    <tr class="${isChecked ? "is-selected" : ""}" data-row-id="${escapeHtml(row.id || row.mask)}">
                      <td class="td-chk"><input type="checkbox" class="script-dlg-row-chk" data-table="${escapeHtml(ctrl.id)}" data-row-id="${escapeHtml(row.id || row.mask)}" ${isChecked ? "checked" : ""} /></td>
                      ${cols.map((col) => `<td>${escapeHtml(String(row[col.id] ?? ""))}</td>`).join("")}
                    </tr>
                  `;
                }).join("")}
              </tbody>
            </table>
          </div>
          <div class="script-dlg-table-footer">
            <div class="script-dlg-table-count">${rows.length} ${ctrl.count_label || "entries"}</div>
            <div class="script-dlg-table-actions">
              ${actions.map((act) => `
                <button type="button" class="btn btn-secondary script-dlg-table-act-btn"
                  data-table="${escapeHtml(ctrl.id)}"
                  data-action-id="${escapeHtml(act.id)}"
                  ${act.prompt ? `data-prompt="${escapeHtml(act.prompt)}"` : ""}
                  ${act.confirm ? `data-confirm="${escapeHtml(act.confirm)}"` : ""}>
                  ${escapeHtml(act.label)}
                </button>
              `).join("")}
            </div>
          </div>
        </div>
      `;
    }

    case "stats": {
      const summary = ctrl.summary || [];
      const users = ctrl.users || [];
      return `
        <div class="script-dlg-stats-section">
          <div class="script-dlg-stats-cards">
            ${summary.map((s) => {
              let extraCls = "";
              const lbl = String(s.label || "");
              if (lbl.includes("Ops")) extraCls = "stat-op";
              else if (lbl.includes("Halfops")) extraCls = "stat-hop";
              else if (lbl.includes("Voices")) extraCls = "stat-voice";
              else if (lbl.includes("Regulars")) extraCls = "stat-reg";
              return `
                <div class="script-dlg-stat-card ${extraCls}">
                  <div class="script-dlg-stat-val">${escapeHtml(String(s.value ?? 0))}</div>
                  <div class="script-dlg-stat-label">${escapeHtml(lbl)}</div>
                </div>
              `;
            }).join("")}
          </div>
          ${users.length > 0 ? `
            <div class="script-dlg-users-header">
              <div class="script-dlg-box-title" style="margin-bottom:0;">User Breakdown (${users.length})</div>
              <input type="text" class="form-input script-dlg-user-search" data-id="stats_user_search" placeholder="Search nick or host..." value="${escapeHtml(activeDialog.drafts["stats_user_search"] || "")}" />
            </div>
            <div class="script-dlg-users-scroll">
              <div class="script-dlg-users-grid">
                ${(() => {
                  const q = (activeDialog.drafts["stats_user_search"] || "").toLowerCase().trim();
                  return users.map((u) => {
                    const nick = String(u.nick || "").toLowerCase();
                    const host = String(u.host || "").toLowerCase();
                    const isVisible = !q || nick.includes(q) || host.includes(q);
                    let prefixCls = "prefix-reg";
                    if (u.prefix === "@" || u.prefix === "~" || u.prefix === "&") prefixCls = "prefix-op";
                    else if (u.prefix === "%") prefixCls = "prefix-hop";
                    else if (u.prefix === "+") prefixCls = "prefix-voice";
                    return `
                      <div class="script-dlg-user-card" data-nick="${escapeHtml(nick)}" style="display: ${isVisible ? "flex" : "none"};">
                        <span class="script-dlg-user-badge ${prefixCls}">${escapeHtml(u.prefix || "")}</span>
                        <span class="script-dlg-user-nick">${escapeHtml(u.nick || "")}</span>
                        ${u.host ? `<span class="script-dlg-user-host" title="${escapeHtml(u.host)}">${escapeHtml(u.host)}</span>` : ""}
                      </div>
                    `;
                  }).join("");
                })()}
              </div>
            </div>
          ` : ""}
        </div>
      `;
    }

    default:
      return "";
  }
}

function bindDomEvents(container) {
  // Backdrop click with mousedown check
  const backdrop = container.querySelector("#" + BACKDROP_ID);
  backdrop?.addEventListener("mousedown", (e) => {
    pressStartedOnBackdrop = e.target === backdrop;
  });
  backdrop?.addEventListener("click", (e) => {
    if (pressStartedOnBackdrop && e.target === backdrop) {
      closeOpenScriptDialog(true);
    }
    pressStartedOnBackdrop = false;
  });

  // Close X button
  container.querySelector(".script-dlg-close-x")?.addEventListener("click", () => {
    closeOpenScriptDialog(true);
  });

  // Tab switching
  container.querySelectorAll(".script-dlg-tab").forEach((tabBtn) => {
    tabBtn.addEventListener("click", () => {
      collectDraftsFromDOM();
      activeDialog.activeTab = tabBtn.dataset.tab;
      renderScriptDialog();
      dispatchEvent({ type: "tab_change", tab: activeDialog.activeTab });
    });
  });

  // User breakdown filter
  const userSearch = container.querySelector(".script-dlg-user-search");
  if (userSearch) {
    const applyUserFilter = (q) => {
      for (const card of container.querySelectorAll(".script-dlg-user-card")) {
        const nick = card.dataset.nick || "";
        const host = card.querySelector(".script-dlg-user-host")?.textContent.toLowerCase() || "";
        card.style.display = (!q || nick.includes(q) || host.includes(q)) ? "flex" : "none";
      }
    };
    userSearch.addEventListener("input", (e) => {
      const q = e.target.value.toLowerCase().trim();
      activeDialog.drafts["stats_user_search"] = e.target.value;
      applyUserFilter(q);
    });
    // Immediately apply saved search filter if any
    const initialQuery = (userSearch.value || "").toLowerCase().trim();
    if (initialQuery) {
      applyUserFilter(initialQuery);
    }
  }

  // Footer Buttons
  container.querySelectorAll(".script-dlg-btn").forEach((btn) => {
    btn.addEventListener("click", () => {
      const btnId = btn.dataset.btnId;
      collectDraftsFromDOM();
      if (btnId === "close" || btnId === "cancel") {
        closeOpenScriptDialog(true);
      } else {
        dispatchEvent({ type: "button", button_id: btnId });
      }
    });
  });

  // Inputs / Checkboxes change tracking
  const handleInputChange = (el) => {
    const id = el.dataset.id;
    if (el.type === "checkbox") {
      activeDialog.drafts[id] = el.checked;

      // When an enable checkbox for mode_inputs changes, immediately toggle target input disabled
      if (id.endsWith("_enable")) {
        const baseId = id.slice(0, -7);
        const targetInput = container.querySelector(`input[data-id="${CSS.escape(baseId)}"]`);
        if (targetInput) {
          targetInput.disabled = !el.checked;
          if (el.checked) targetInput.focus();
        }
      }
    } else {
      activeDialog.drafts[id] = el.value;
    }

    // Live update topic preview
    if (id === "topic") {
      const previewEl = container.querySelector("#script-dlg-topic-preview");
      if (previewEl) {
        previewEl.innerHTML = el.value ? parseIrcToHtml(el.value) : "<em>Empty</em>";
      }
      const barEl = container.querySelector(".script-dlg-progress-bar");
      const charsLeftEl = container.querySelector(".script-dlg-chars-left");
      const maxLen = parseInt(el.getAttribute("maxlength") || "500", 10);
      const rem = Math.max(0, maxLen - el.value.length);
      const pct = Math.min(100, Math.round((el.value.length / maxLen) * 100));
      if (barEl) {
        barEl.style.width = pct + "%";
        if (pct > 90) barEl.classList.add("is-warning");
        else barEl.classList.remove("is-warning");
      }
      if (charsLeftEl) charsLeftEl.textContent = `${rem} remaining`;
    }
  };

  container.querySelectorAll("input[data-id], textarea[data-id]").forEach((el) => {
    el.addEventListener("input", () => handleInputChange(el));
    if (el.type === "checkbox") {
      el.addEventListener("change", () => handleInputChange(el));
    }
  });

  // Topic history select
  const histSelect = container.querySelector("#script-dlg-topic-history-select");
  histSelect?.addEventListener("change", () => {
    const idx = parseInt(histSelect.value, 10);
    const spec = activeDialog.spec;
    const ctrl = (spec.controls || []).find((c) => c.type === "topic_editor")
      || (spec.tab_contents && spec.tab_contents[activeDialog.activeTab] || []).find((c) => c.type === "topic_editor");
    if (ctrl && ctrl.history && ctrl.history[idx]) {
      const chosenTopic = ctrl.history[idx].topic || "";
      activeDialog.drafts[ctrl.id || "topic"] = chosenTopic;
      const topicInput = container.querySelector("#script-dlg-topic-input");
      if (topicInput) {
        topicInput.value = chosenTopic;
        topicInput.dispatchEvent(new Event("input"));
      }
    }
  });

  // Action buttons (e.g. Set topic, Clear topic)
  container.querySelectorAll(".script-dlg-action-btn").forEach((btn) => {
    btn.addEventListener("click", () => {
      collectDraftsFromDOM();
      const action = btn.dataset.action;
      const control = btn.dataset.control;
      if (action === "clear_topic") {
        activeDialog.drafts[control] = "";
        const topicInput = container.querySelector("#script-dlg-topic-input");
        if (topicInput) {
          topicInput.value = "";
          topicInput.dispatchEvent(new Event("input"));
        }
      }
      dispatchEvent({ type: "action", action, control });
    });
  });

  // Table row select
  container.querySelectorAll(".script-dlg-row-chk").forEach((chk) => {
    chk.addEventListener("change", () => {
      const tableId = chk.dataset.table;
      const rowId = chk.dataset.rowId;
      const key = tableId + "_selected";
      const sel = new Set(activeDialog.drafts[key] || []);
      if (chk.checked) sel.add(rowId);
      else sel.delete(rowId);
      activeDialog.drafts[key] = Array.from(sel);

      const tr = chk.closest("tr");
      if (tr) {
        if (chk.checked) tr.classList.add("is-selected");
        else tr.classList.remove("is-selected");
      }
    });
  });

  // Table select all
  container.querySelectorAll(".script-dlg-select-all").forEach((allChk) => {
    allChk.addEventListener("change", () => {
      const tableId = allChk.dataset.table;
      const key = tableId + "_selected";
      const sel = new Set();
      const rowChks = container.querySelectorAll(`.script-dlg-row-chk[data-table="${tableId}"]`);
      rowChks.forEach((chk) => {
        chk.checked = allChk.checked;
        const rowId = chk.dataset.rowId;
        if (allChk.checked) sel.add(rowId);
        const tr = chk.closest("tr");
        if (tr) {
          if (allChk.checked) tr.classList.add("is-selected");
          else tr.classList.remove("is-selected");
        }
      });
      activeDialog.drafts[key] = Array.from(sel);
    });
  });

  // Table header sorting
  container.querySelectorAll(".script-dlg-sortable-th").forEach((th) => {
    th.addEventListener("click", () => {
      const col = th.dataset.col;
      const tableId = th.dataset.table;
      if (!col || !tableId) return;

      const spec = activeDialog.spec || {};
      const allCtrls = [
        ...(spec.controls || []),
        ...(spec.tab_contents?.[activeDialog.activeTab] || []),
      ];
      const tblCtrl = allCtrls.find((c) => c.type === "table" && c.id === tableId);
      if (tblCtrl && Array.isArray(tblCtrl.rows)) {
        const isDesc = tblCtrl.sort_by === col ? !tblCtrl.sort_desc : false;
        tblCtrl.sort_by = col;
        tblCtrl.sort_desc = isDesc;
        tblCtrl.rows.sort((a, b) => {
          const va = String(a[col] ?? "");
          const vb = String(b[col] ?? "");
          const cmp = va.localeCompare(vb, undefined, { numeric: true, sensitivity: "base" });
          return isDesc ? -cmp : cmp;
        });
        collectDraftsFromDOM();
        renderScriptDialog();
        dispatchEvent({ type: "sort", table: tableId, col, desc: isDesc });
      }
    });
  });

  // Table actions (Add, Remove) with askText modal prompt
  container.querySelectorAll(".script-dlg-table-act-btn").forEach((btn) => {
    btn.addEventListener("click", async () => {
      const tableId = btn.dataset.table;
      const actionId = btn.dataset.actionId;
      const promptMsg = btn.dataset.prompt;
      const confirmMsg = btn.dataset.confirm;
      const selected = activeDialog.drafts[tableId + "_selected"] || [];

      let val = null;
      if (promptMsg) {
        val = await askText(promptMsg);
        if (val === null || !val.trim()) return; // cancelled
      }

      if (confirmMsg) {
        if (actionId === "remove" && selected.length === 0) {
          window.alert("Please select one or more items first.");
          return;
        }
        if (!window.confirm(confirmMsg)) return; // cancelled
      }

      collectDraftsFromDOM();
      dispatchEvent({
        type: "table_action",
        table: tableId,
        action: actionId,
        value: val ? val.trim() : null,
        selected,
      });
    });
  });
}
