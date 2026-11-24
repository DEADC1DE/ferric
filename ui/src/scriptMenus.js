// Entries the loaded scripts add to a popup (plan M5). The backend lists them for a popup
// and runs the one that is clicked, with the answers to its prompts.

import { escapeHtml } from "./utils/ircColors.js";
import { askText } from "./textPrompt.js";

const SCRIPT_MENU_WINDOWS = ["status", "channel", "query", "nicklist", "menubar"];

/**
 * The entries the loaded scripts add to a popup of this window kind. Empty when there are
 * none or the backend refuses; a popup then shows only its own entries.
 */
export async function fetchScriptMenuNodes(invoke, window, { network, nick = "", nicks = [], channel = "", onError = null }) {
  if (!SCRIPT_MENU_WINDOWS.includes(window) || !network) return [];
  try {
    const nodes = await invoke("script_menu_nodes", { network, window, nick, nicks, channel });
    return Array.isArray(nodes) ? nodes : [];
  } catch (error) {
    // A refused lookup must not look like a popup without script entries.
    if (typeof onError === "function") onError(error);
    return [];
  }
}

/**
 * HTML for the entries, and the command entries in the order of their `data-script-index`.
 * Labels are escaped. A submenu holds its entries in a box that opens to the right.
 */
export function renderScriptMenuNodes(nodes) {
  const commands = [];
  // The entries of each submenu, by id. Every submenu opens as a flyout beside its entry on hover,
  // to any depth.
  const flyouts = new Map();
  const renderList = (list) => list.map((node) => {
    if (node.kind === "separator") return '<div class="context-menu-separator"></div>';
    if (node.kind === "submenu") {
      const id = flyouts.size;
      flyouts.set(id, "");
      flyouts.set(id, renderList(node.children || []));
      return `<div class="context-menu-item has-submenu" data-script-submenu="${id}">${escapeHtml(node.label)}</div>`;
    }
    const index = commands.push(node) - 1;
    return `<div class="context-menu-item" data-script-index="${index}">${escapeHtml(node.label)}</div>`;
  }).join("");
  const html = renderList(nodes);
  return { html, commands, flyouts };
}

/**
 * Makes the command entries of a rendered popup run. The popup closes first, then `run(node)`
 * is awaited.
 */
export function bindScriptMenuItems(menu, commands, { onClose, run }) {
  menu.querySelectorAll("[data-script-index]").forEach((el) => {
    el.addEventListener("click", async () => {
      const node = commands[Number(el.dataset.scriptIndex)];
      onClose();
      if (node) await run(node);
    });
  });
}

/**
 * Asks each prompt of a command entry in order, then runs it in the window the popup came from.
 * A cancelled prompt runs nothing. `ask` shows one prompt and returns its answer or null.
 */
export async function runScriptMenuNode(invoke, node, {
  network,
  buffer,
  nick = "",
  nicks = [],
  channel = "",
  ask = askText,
}) {
  const answers = [];
  for (const question of node.prompts || []) {
    const answer = await ask(question);
    if (answer === null) return false;
    answers.push(String(answer));
  }
  await invoke("run_script_menu_item", {
    network,
    buffer: buffer || "status",
    address: node.address,
    nick,
    nicks,
    channel,
    answers,
  });
  return true;
}
