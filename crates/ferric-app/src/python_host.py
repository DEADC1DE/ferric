# python_host.py - Ferric Python Worker Host
# Runs as a child process of Ferric communicating via JSON lines on a dedicated stdout fd.

import sys
import os
import json
import types
import traceback
import fnmatch
import inspect

# 1. Duplicate descriptor 1 for the NDJSON protocol stream before redirecting sys.stdout.
try:
    _json_out = os.fdopen(os.dup(1), "w", encoding="utf-8", buffering=1)
    # Redirect standard stdout to stderr so user print() calls cannot corrupt the JSON protocol.
    sys.stdout = sys.stderr
except Exception as e:
    sys.stderr.write(f"Failed to initialize JSON protocol stream: {e}\n")
    sys.exit(1)

# Active execution turn buffers
_current_effects = []
_current_notices = []
_current_dialog_ops = []
_current_context = None
_current_script_name = None

# Active dialog tracking: lower_dialog_id -> script_name
_open_dialogs = {}

# Script registry
# script_name -> { "name": str, "module": module, "aliases": dict, "events": list, "menus": list, "dialogs": dict, "on_load": fn, "on_unload": fn }
_scripts = {}

# Current script being loaded (temporary dict)
_loading_script = None

def _emit_effect(eff):
    _current_effects.append(eff)

def _emit_notice(text):
    _current_notices.append(str(text))

# Define the virtual 'ferric' module
class FerricModule(types.ModuleType):
    """The ferric API module exposed to scripts."""

    def __init__(self):
        super().__init__("ferric")

    # --- Actions ---
    def echo(self, text, target="default"):
        _emit_effect({"type": "echo", "text": str(text), "target": str(target)})

    def msg(self, target, text):
        _emit_effect({"type": "msg", "target": str(target), "text": str(text)})

    def notice(self, target, text):
        _emit_effect({"type": "notice", "target": str(target), "text": str(text)})

    def action(self, target, text):
        _emit_effect({"type": "action", "target": str(target), "text": str(text)})

    def join(self, channel, key=None):
        _emit_effect({"type": "join", "channel": str(channel), "key": str(key) if key is not None else None})

    def part(self, channel, reason=None):
        _emit_effect({"type": "part", "channel": str(channel), "reason": str(reason) if reason is not None else None})

    def kick(self, channel, nick, reason=None):
        _emit_effect({"type": "kick", "channel": str(channel), "nick": str(nick), "reason": str(reason) if reason is not None else None})

    def mode(self, target, modes):
        _emit_effect({"type": "mode", "target": str(target), "modes": str(modes)})

    def nick(self, new_nick):
        _emit_effect({"type": "nick", "nick": str(new_nick)})

    def whois(self, nick):
        _emit_effect({"type": "whois", "nick": str(nick)})

    def query(self, nick, text=None):
        _emit_effect({"type": "query", "nick": str(nick), "text": str(text) if text is not None else None})

    def raw(self, line):
        _emit_effect({"type": "raw", "line": str(line)})

    def context(self):
        return _current_context

    # --- Dialog API ---
    def dialog_open(self, dialog_id, title, spec):
        global _current_dialog_ops, _open_dialogs
        did = str(dialog_id).strip()
        owner = _current_script_name or (_loading_script["name"] if _loading_script else "unknown")
        _open_dialogs[did.lower()] = owner
        _current_dialog_ops.append({
            "op": "open",
            "id": did,
            "title": str(title),
            "spec": spec
        })

    def dialog_update(self, dialog_id, title=None, spec=None):
        global _current_dialog_ops
        did = str(dialog_id).strip()
        op = {"op": "update", "id": did}
        if title is not None:
            op["title"] = str(title)
        if spec is not None:
            op["spec"] = spec
        _current_dialog_ops.append(op)

    def dialog_close(self, dialog_id):
        global _current_dialog_ops, _open_dialogs
        did = str(dialog_id).strip()
        _open_dialogs.pop(did.lower(), None)
        _current_dialog_ops.append({
            "op": "close",
            "id": did
        })

    # --- Decorators ---
    def on(self, event_name, match="*", target="*"):
        """Decorator: register an event handler.
        Supported events: text, action, notice, join, part, quit, nick, kick, topic, mode, connect, disconnect, error, ctcpreply.
        """
        def decorator(fn):
            if _loading_script is not None:
                _loading_script["events"].append({
                    "event": str(event_name).strip().lower(),
                    "match": str(match) if match is not None else "*",
                    "target": str(target) if target is not None else "*",
                    "fn": fn
                })
            return fn
        return decorator

    def alias(self, name):
        """Decorator: register a slash command / alias handler."""
        def decorator(fn):
            if _loading_script is not None:
                clean_name = str(name).strip().lstrip("/").lower()
                _loading_script["aliases"][clean_name] = fn
            return fn
        return decorator

    def menu(self, window, label, prompts=None):
        """Decorator: register a popup menu item."""
        def decorator(fn):
            if _loading_script is not None:
                menu_id = f"{_loading_script['name']}:{len(_loading_script['menus'])}"
                _loading_script["menus"].append({
                    "id": menu_id,
                    "window": str(window).strip().lower(),
                    "label": str(label),
                    "prompts": [str(p) for p in prompts] if prompts else [],
                    "fn": fn
                })
            return fn
        return decorator

    def on_dialog(self, dialog_id):
        """Decorator: register a dialog event handler."""
        def decorator(fn):
            if _loading_script is not None:
                clean_id = str(dialog_id).strip().lower()
                _loading_script["dialogs"][clean_id] = fn
            return fn
        return decorator

    def on_load(self, fn):
        """Decorator: runs when the script is loaded."""
        if _loading_script is not None:
            _loading_script["on_load"] = fn
        return fn

    def on_unload(self, fn):
        """Decorator: runs when the script is unloaded."""
        if _loading_script is not None:
            _loading_script["on_unload"] = fn
        return fn

# Register virtual ferric module in sys.modules
ferric = FerricModule()
sys.modules["ferric"] = ferric

def _invoke_fn(fn, *args):
    """Invokes fn with as many positional arguments as its signature accepts without re-invoking on errors."""
    call_args = args
    try:
        sig = inspect.signature(fn)
        pos_params = [
            p for p in sig.parameters.values()
            if p.kind in (inspect.Parameter.POSITIONAL_ONLY, inspect.Parameter.POSITIONAL_OR_KEYWORD)
        ]
        has_varargs = any(p.kind == inspect.Parameter.VAR_POSITIONAL for p in sig.parameters.values())
        if not has_varargs:
            call_args = args[:len(pos_params)]
    except Exception:
        pass
    return fn(*call_args)

def _wildcard_match(pattern, text):
    if pattern == "*" or pattern == "":
        return True
    return fnmatch.fnmatchcase(str(text).lower(), str(pattern).lower())

def _collect_all_registrations():
    events = set()
    aliases = set()
    menus = []
    for sdata in _scripts.values():
        for entry in sdata["events"]:
            events.add(entry["event"])
        for alias in sdata["aliases"].keys():
            aliases.add(alias)
        for m in sdata["menus"]:
            menus.append({
                "id": m["id"],
                "window": m["window"],
                "label": m["label"],
                "prompts": m["prompts"]
            })
    return sorted(list(events)), sorted(list(aliases)), menus

def handle_load(name, content):
    global _loading_script, _current_effects, _current_notices, _current_dialog_ops
    norm_name = str(name).strip().lower()

    try:
        code = compile(content, norm_name, "exec")
    except Exception as e:
        err_msg = traceback.format_exc()
        all_events, all_aliases, all_menus = _collect_all_registrations()
        return {
            "ok": False,
            "error": str(e),
            "notices": [f"Error in {name}: {e}\n{err_msg}"],
            "effects": [],
            "dialog_ops": [],
            "aliases": all_aliases,
            "events": all_events,
            "menus": all_menus
        }

    new_script = {
        "name": norm_name,
        "module": None,
        "aliases": {},
        "events": [],
        "menus": [],
        "dialogs": {},
        "on_load": None,
        "on_unload": None
    }
    module = types.ModuleType(norm_name)
    module.__file__ = norm_name
    module.ferric = ferric
    new_script["module"] = module

    _loading_script = new_script
    _current_effects = []
    _current_notices = []
    _current_dialog_ops = []

    try:
        exec(code, module.__dict__)
    except Exception as e:
        _loading_script = None
        err_msg = traceback.format_exc()
        all_events, all_aliases, all_menus = _collect_all_registrations()
        return {
            "ok": False,
            "error": str(e),
            "notices": [f"Error in {name}: {e}\n{err_msg}"],
            "effects": [],
            "dialog_ops": [],
            "aliases": all_aliases,
            "events": all_events,
            "menus": all_menus
        }

    _loading_script = None

    # Previous version exists: invoke previous on_unload before overwriting
    if norm_name in _scripts and _scripts[norm_name].get("on_unload"):
        try:
            _invoke_fn(_scripts[norm_name]["on_unload"])
        except Exception as e:
            _current_notices.append(f"Error in {name} previous on_unload: {e}")

    _scripts[norm_name] = new_script

    # Run on_load
    if new_script["on_load"]:
        try:
            _invoke_fn(new_script["on_load"])
        except Exception as e:
            _current_notices.append(f"Error in {name} on_load: {e}")

    all_events, all_aliases, all_menus = _collect_all_registrations()

    effects = list(_current_effects)
    notices = list(_current_notices)
    dialog_ops = list(_current_dialog_ops)
    _current_effects = []
    _current_notices = []
    _current_dialog_ops = []

    return {
        "ok": True,
        "aliases": all_aliases,
        "events": all_events,
        "menus": all_menus,
        "effects": effects,
        "notices": notices,
        "dialog_ops": dialog_ops
    }

def handle_unload(name):
    global _current_effects, _current_notices, _current_dialog_ops, _open_dialogs
    norm_name = str(name).strip().lower()
    if norm_name not in _scripts:
        all_events, all_aliases, all_menus = _collect_all_registrations()
        return {"ok": True, "aliases": all_aliases, "events": all_events, "menus": all_menus, "effects": [], "notices": [], "dialog_ops": []}

    _current_effects = []
    _current_notices = []
    _current_dialog_ops = []

    # Automatically close any open dialogs owned by this script
    to_close = [did for did, owner in list(_open_dialogs.items()) if owner == norm_name]
    for did in to_close:
        _open_dialogs.pop(did, None)
        _current_dialog_ops.append({"op": "close", "id": did})

    on_unload = _scripts[norm_name].get("on_unload")
    if on_unload:
        try:
            _invoke_fn(on_unload)
        except Exception as e:
            _current_notices.append(f"Error in {name} on_unload: {e}")

    _scripts.pop(norm_name, None)

    all_events, all_aliases, all_menus = _collect_all_registrations()

    effects = list(_current_effects)
    notices = list(_current_notices)
    dialog_ops = list(_current_dialog_ops)
    _current_effects = []
    _current_notices = []
    _current_dialog_ops = []

    return {
        "ok": True,
        "aliases": all_aliases,
        "events": all_events,
        "menus": all_menus,
        "effects": effects,
        "notices": notices,
        "dialog_ops": dialog_ops
    }

def handle_event(event_type, event, context):
    global _current_effects, _current_notices, _current_dialog_ops, _current_context, _current_script_name
    _current_effects = []
    _current_notices = []
    _current_dialog_ops = []
    _current_context = context

    if isinstance(event, dict) and len(event) == 1:
        first_val = next(iter(event.values()))
        if isinstance(first_val, dict):
            event = dict(first_val)
        else:
            event = dict(event)
    elif isinstance(event, dict):
        event = dict(event)
    else:
        event = {}

    etype = str(event_type).strip().lower()
    event.setdefault("type", etype)

    if "target" in event and "channel" not in event:
        t = str(event["target"])
        if t.startswith(('#', '&', '+', '!')):
            event["channel"] = t
    elif "channel" in event and "target" not in event:
        event["target"] = event["channel"]

    if "text" in event and "message" not in event:
        event["message"] = event["text"]
    elif "message" in event and "text" not in event:
        event["text"] = event["message"]

    text = (
        event.get("text")
        or event.get("message")
        or event.get("reason")
        or event.get("modes")
        or event.get("param")
        or ""
    )
    target = (
        event.get("target")
        or event.get("channel")
        or event.get("chan")
        or ""
    )

    for sname, sdata in list(_scripts.items()):
        for handler in sdata["events"]:
            if handler["event"] != etype and handler["event"] != "*":
                continue
            if not _wildcard_match(handler["match"], text):
                continue
            if not _wildcard_match(handler["target"], target):
                continue
            _current_script_name = sname
            try:
                _invoke_fn(handler["fn"], event, context)
            except Exception as e:
                _current_notices.append(f"Error in {sname} @ferric.on({etype}): {e}")
            finally:
                _current_script_name = None

    effects = list(_current_effects)
    notices = list(_current_notices)
    dialog_ops = list(_current_dialog_ops)
    _current_effects = []
    _current_notices = []
    _current_dialog_ops = []
    _current_context = None

    return {"effects": effects, "notices": notices, "dialog_ops": dialog_ops}

def handle_alias(name, args, context):
    global _current_effects, _current_notices, _current_dialog_ops, _current_context, _current_script_name
    _current_effects = []
    _current_notices = []
    _current_dialog_ops = []
    _current_context = context

    alias_name = str(name).strip().lstrip("/").lower()
    found = False

    for sname, sdata in list(_scripts.items()):
        if alias_name in sdata["aliases"]:
            found = True
            handler = sdata["aliases"][alias_name]
            _current_script_name = sname
            try:
                _invoke_fn(handler, args, context)
            except Exception as e:
                _current_notices.append(f"Error in {sname} /{alias_name}: {e}")
            finally:
                _current_script_name = None

    effects = list(_current_effects)
    notices = list(_current_notices)
    dialog_ops = list(_current_dialog_ops)
    _current_effects = []
    _current_notices = []
    _current_dialog_ops = []
    _current_context = None

    return {"found": found, "effects": effects, "notices": notices, "dialog_ops": dialog_ops}

def handle_dialog_event(dialog_id, event, context):
    global _current_effects, _current_notices, _current_dialog_ops, _current_context, _current_script_name
    _current_effects = []
    _current_notices = []
    _current_dialog_ops = []
    _current_context = context

    did = str(dialog_id).strip().lower()
    found = False

    for sname, sdata in list(_scripts.items()):
        if did in sdata.get("dialogs", {}):
            found = True
            handler = sdata["dialogs"][did]
            _current_script_name = sname
            try:
                _invoke_fn(handler, event, context)
            except Exception as e:
                _current_notices.append(f"Error in {sname} dialog '{dialog_id}': {e}")
            finally:
                _current_script_name = None

    effects = list(_current_effects)
    notices = list(_current_notices)
    dialog_ops = list(_current_dialog_ops)
    _current_effects = []
    _current_notices = []
    _current_dialog_ops = []
    _current_context = None

    return {"ok": found, "effects": effects, "notices": notices, "dialog_ops": dialog_ops}

def handle_menu_nodes(window, context, args):
    win = str(window).strip().lower()
    nodes = []
    for sname, sdata in _scripts.items():
        for m in sdata["menus"]:
            if m["window"] == win or m["window"] == "*":
                nodes.append({
                    "id": m["id"],
                    "label": m["label"],
                    "prompts": m["prompts"]
                })
    return {"nodes": nodes}

def handle_run_menu_item(menu_id, context, args, answers):
    global _current_effects, _current_notices, _current_dialog_ops, _current_context, _current_script_name
    _current_effects = []
    _current_notices = []
    _current_dialog_ops = []
    _current_context = context

    found = False
    for sname, sdata in _scripts.items():
        for m in sdata["menus"]:
            if m["id"] == menu_id:
                found = True
                _current_script_name = sname
                try:
                    _invoke_fn(m["fn"], context, args, answers)
                except Exception as e:
                    _current_notices.append(f"Error in {sname} menu '{m['label']}': {e}")
                finally:
                    _current_script_name = None
                break
        if found:
            break

    effects = list(_current_effects)
    notices = list(_current_notices)
    dialog_ops = list(_current_dialog_ops)
    _current_effects = []
    _current_notices = []
    _current_dialog_ops = []
    _current_context = None

    if not found:
        return {"ok": False, "effects": [], "notices": [f"Menu item '{menu_id}' not found"], "dialog_ops": []}

    return {"ok": True, "effects": effects, "notices": notices, "dialog_ops": dialog_ops}

def main():
    while True:
        try:
            line = sys.stdin.readline()
            if not line:
                break
            line = line.strip()
            if not line:
                continue

            try:
                msg = json.loads(line)
            except Exception as e:
                _json_out.write(json.dumps({"ok": False, "error": f"Invalid JSON: {e}"}) + "\n")
                _json_out.flush()
                continue

            cmd = msg.get("cmd")
            ctx = msg.get("context", {})
            if "channel_state" in msg and ctx is not None:
                ctx["channel_state"] = msg.get("channel_state")

            if cmd == "ping":
                resp = {"ok": True}
            elif cmd == "load":
                resp = handle_load(msg.get("name", ""), msg.get("content", ""))
            elif cmd == "unload":
                resp = handle_unload(msg.get("name", ""))
            elif cmd == "event":
                resp = handle_event(msg.get("event_type", ""), msg.get("event", {}), ctx)
            elif cmd == "alias":
                resp = handle_alias(msg.get("name", ""), msg.get("args", []), ctx)
            elif cmd == "dialog_event":
                resp = handle_dialog_event(msg.get("id", ""), msg.get("event", {}), ctx)
            elif cmd == "menu_nodes":
                resp = handle_menu_nodes(msg.get("window", ""), ctx, msg.get("args", []))
            elif cmd == "run_menu_item":
                resp = handle_run_menu_item(msg.get("id", ""), ctx, msg.get("args", []), msg.get("answers", []))
            else:
                resp = {"ok": False, "error": f"Unknown command: {cmd}"}

            _json_out.write(json.dumps(resp) + "\n")
            _json_out.flush()
        except KeyboardInterrupt:
            break
        except Exception as e:
            sys.stderr.write(f"Worker exception: {traceback.format_exc()}\n")
            try:
                _json_out.write(json.dumps({"ok": False, "error": str(e), "effects": [], "notices": [str(e)], "dialog_ops": []}) + "\n")
                _json_out.flush()
            except Exception:
                pass

if __name__ == "__main__":
    main()
