# channelcentral.py - Channel Central for Ferric IRC
# Declarative 6-tab modal GUI via ferric.dialog_* API
# Aliases: /channel, /cc, /ccmodes, /ccban, /ccexcept, /ccinvite, /findban, /ccclear, /cctopic, /cctopics, /ccstats, /ccrefresh

import fnmatch
import json
import os
import sys
import time
import ferric

_SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
_HIST_PATH = os.path.join(_SCRIPT_DIR, "channel_central_topics.json")
_MAX_TOPIC_HIST = 50
_DEFAULT_TOPICLEN = 500
_MODES_PER_LINE = 4
_STANDARD_FLAGS = ("t", "n", "i", "p", "s", "m")
_LIST_MODES = {"b": "bans", "e": "excepts", "I": "invites"}
_PARAM_MODES_ALWAYS = set("beIkovhqa")

# Persistent topic history: "network|channel" -> [{"topic": str, "by": str, "at": int}]
_topic_hist = {}

# Active dialog tracking
_open_dialog = {
    "network": "",
    "channel": "",
    "active_tab": "modes",
}

# Internal channel cache: (network_lower, channel_lower) -> dict
_channels = {}


def _key(network, channel):
    return (str(network or "").lower(), str(channel or "").lower())


def _hist_key(network, channel):
    return f"{str(network or '').lower()}|{str(channel or '').lower()}"


def _format_ts(ts):
    if not ts:
        return "-"
    try:
        return time.strftime("%Y-%m-%d %H:%M:%S", time.localtime(int(ts)))
    except Exception:
        return str(ts)


def _load_topic_hist():
    global _topic_hist
    try:
        if os.path.isfile(_HIST_PATH):
            with open(_HIST_PATH, "r", encoding="utf-8") as fh:
                data = json.load(fh)
            if isinstance(data, dict):
                _topic_hist = data
    except Exception:
        _topic_hist = {}


def _save_topic_hist():
    try:
        with open(_HIST_PATH, "w", encoding="utf-8") as fh:
            json.dump(_topic_hist, fh, ensure_ascii=False, indent=2)
    except Exception:
        pass


def _record_topic(network, channel, topic, setter=""):
    if not topic or not channel:
        return
    hk = _hist_key(network, channel)
    entries = _topic_hist.setdefault(hk, [])
    now = int(time.time())
    entries = [e for e in entries if e.get("topic") != topic]
    entries.insert(0, {"topic": topic, "by": setter or "", "at": now})
    _topic_hist[hk] = entries[:_MAX_TOPIC_HIST]
    _save_topic_hist()


def _get_chan_state(network, channel, cs=None):
    k = _key(network, channel)
    st = _channels.get(k)
    if st is None:
        st = {
            "network": network,
            "channel": channel,
            "flags": set(),
            "key": None,
            "limit": None,
            "topic": "",
            "topic_by": "",
            "topic_ts": 0,
            "bans": {},
            "excepts": {},
            "invites": {},
            "users": {},
            "my_prefix": None,
        }
        _channels[k] = st

    if cs:
        if cs.get("channel_flags") is not None:
            st["flags"] = set(cs["channel_flags"])
        if "channel_key" in cs:
            st["key"] = cs.get("channel_key")
        if "channel_limit" in cs:
            st["limit"] = cs.get("channel_limit")
        if cs.get("topic") is not None:
            st["topic"] = cs["topic"] or ""
        if cs.get("topic_set_by") is not None:
            st["topic_by"] = cs["topic_set_by"] or ""
        if "my_prefix" in cs:
            st["my_prefix"] = cs.get("my_prefix")
        if "my_prefix_modes" in cs:
            st["my_prefix_modes"] = cs.get("my_prefix_modes")

        # Bans
        if "bans" in cs and isinstance(cs["bans"], list):
            st["bans"] = {
                b["mask"].lower(): {"mask": b["mask"], "by": b.get("by", ""), "ts": b.get("at", 0)}
                for b in cs["bans"] if "mask" in b
            }
        # Excepts
        if "excepts" in cs and isinstance(cs["excepts"], list):
            st["excepts"] = {
                e["mask"].lower(): {"mask": e["mask"], "by": e.get("by", ""), "ts": e.get("at", 0)}
                for e in cs["excepts"] if "mask" in e
            }
        # Invites
        if "invites" in cs and isinstance(cs["invites"], list):
            st["invites"] = {
                i["mask"].lower(): {"mask": i["mask"], "by": i.get("by", ""), "ts": i.get("at", 0)}
                for i in cs["invites"] if "mask" in i
            }
        # Users
        if "users" in cs and isinstance(cs["users"], dict):
            st["users"] = cs["users"]

    return st


def _send_batched_modes(channel, sign_letter, items):
    sign = sign_letter[0]
    letter = sign_letter[1]
    for i in range(0, len(items), _MODES_PER_LINE):
        chunk = items[i : i + _MODES_PER_LINE]
        ferric.mode(channel, f"{sign}{letter * len(chunk)} {' '.join(chunk)}")


# ---------------------------------------------------------------------------
# Declarative Dialog Spec Builder
# ---------------------------------------------------------------------------

def build_dialog_spec(network, channel, st, active_tab="modes"):
    flags_set = st.get("flags", set())
    key_val = st.get("key")
    limit_val = st.get("limit")
    topic_val = st.get("topic", "")
    topic_by = st.get("topic_by", "")
    topic_ts = st.get("topic_ts", 0)

    bans_map = st.get("bans", {})
    excepts_map = st.get("excepts", {})
    invites_map = st.get("invites", {})
    users_map = st.get("users", {})

    bans_rows = [
        {"id": b["mask"], "mask": b["mask"], "by": b.get("by") or "-", "date": _format_ts(b.get("ts"))}
        for b in sorted(bans_map.values(), key=lambda x: x.get("ts", 0), reverse=True)
    ]
    excepts_rows = [
        {"id": e["mask"], "mask": e["mask"], "by": e.get("by") or "-", "date": _format_ts(e.get("ts"))}
        for e in sorted(excepts_map.values(), key=lambda x: x.get("ts", 0), reverse=True)
    ]
    invites_rows = [
        {"id": i["mask"], "mask": i["mask"], "by": i.get("by") or "-", "date": _format_ts(i.get("ts"))}
        for i in sorted(invites_map.values(), key=lambda x: x.get("ts", 0), reverse=True)
    ]

    # User breakdown
    users_list = list(users_map.values()) if isinstance(users_map, dict) else []
    ops_count = 0
    hops_count = 0
    voices_count = 0
    regs_count = 0
    user_chips = []

    for u in sorted(users_list, key=lambda x: str(x.get("nick", "")).lower()):
        nick = u.get("nick", "")
        prefix = u.get("prefix") or ""
        pmodes = u.get("prefix_modes") or []
        modes = set(u.get("modes") or [])

        is_op = prefix in ("@", "~", "&") or "o" in modes or "o" in pmodes
        is_hop = prefix == "%" or "h" in modes or "h" in pmodes
        is_voice = prefix == "+" or "v" in modes or "v" in pmodes

        if is_op:
            ops_count += 1
            disp_p = "@"
        elif is_hop:
            hops_count += 1
            disp_p = "%"
        elif is_voice:
            voices_count += 1
            disp_p = "+"
        else:
            regs_count += 1
            disp_p = ""

        user_chips.append({
            "nick": nick,
            "prefix": disp_p,
            "host": u.get("host") or "",
        })

    total_users = len(users_list)
    cur_modes_str = "+" + "".join(sorted(flags_set))
    if key_val:
        cur_modes_str += f" +k {key_val}"
    if limit_val:
        cur_modes_str += f" +l {limit_val}"

    # Persistent topic history
    hk = _hist_key(network, channel)
    persisted_topics = _topic_hist.get(hk, [])

    my_pfx = st.get("my_prefix") or ""
    my_pmodes = st.get("my_prefix_modes") or []
    can_op = my_pfx in ("@", "~", "&") or any(m in ("o", "a", "q") for m in my_pmodes)

    # Tab 1: Modes
    modes_controls = [
        {
            "type": "info_box",
            "title": "Current channel information",
            "items": [
                {"label": "Active channel", "value": channel},
                {"label": "Active network", "value": network or "Unknown"},
                {"label": "Current modes", "value": cur_modes_str},
                {"label": "Amount of ops (@)", "value": f"{ops_count} / {total_users}"},
                {"label": "Available modes", "value": "+tnipsmklbeI"},
            ],
        },
        {
            "type": "checkbox_grid",
            "title": "Standard modes",
            "columns": 2,
            "items": [
                {"id": "t", "label": "t - Only ops set topic", "checked": "t" in flags_set, "disabled": not can_op},
                {"id": "n", "label": "n - No ext. messages", "checked": "n" in flags_set, "disabled": not can_op},
                {"id": "i", "label": "i - Invite only", "checked": "i" in flags_set, "disabled": not can_op},
                {"id": "p", "label": "p - Private", "checked": "p" in flags_set, "disabled": not can_op},
                {"id": "s", "label": "s - Secret", "checked": "s" in flags_set, "disabled": not can_op},
                {"id": "m", "label": "m - Moderated", "checked": "m" in flags_set, "disabled": not can_op},
            ],
        },
        {
            "type": "mode_inputs",
            "items": [
                {"id": "k", "label": "k - Channel key (password)", "enabled": bool(key_val), "value": key_val or "", "is_password": True, "disabled": not can_op},
                {"id": "l", "label": "l - Limit channel capacity", "enabled": bool(limit_val), "value": str(limit_val or ""), "disabled": not can_op},
            ],
        },
    ]

    # Tab 2: Bans
    bans_controls = [
        {
            "type": "table",
            "id": "bans_table",
            "columns": [
                {"id": "mask", "label": "Ban Mask"},
                {"id": "by", "label": "Set by"},
                {"id": "date", "label": "Set date"},
            ],
            "rows": bans_rows,
            "count_label": "bans",
            "empty_text": "No bans set in this channel.",
            "actions": [
                {"id": "add", "label": "Add Ban...", "prompt": "Enter ban mask (e.g. *!*@host or nick!*@*):"},
                {"id": "remove", "label": "Remove", "confirm": "Remove selected ban(s)?"},
                {"id": "clear", "label": "Clear All", "confirm": "Remove all bans in this channel?"},
                {"id": "refresh", "label": "Refresh"},
            ],
        }
    ]

    # Tab 3: Excepts
    excepts_controls = [
        {
            "type": "table",
            "id": "excepts_table",
            "columns": [
                {"id": "mask", "label": "Exception Mask"},
                {"id": "by", "label": "Set by"},
                {"id": "date", "label": "Set date"},
            ],
            "rows": excepts_rows,
            "count_label": "exceptions",
            "empty_text": "No exceptions (+e) set in this channel.",
            "actions": [
                {"id": "add", "label": "Add Except...", "prompt": "Enter exception mask (e.g. *!*@host):"},
                {"id": "remove", "label": "Remove", "confirm": "Remove selected exception(s)?"},
                {"id": "clear", "label": "Clear All", "confirm": "Remove all exceptions in this channel?"},
                {"id": "refresh", "label": "Refresh"},
            ],
        }
    ]

    # Tab 4: Invites
    invites_controls = [
        {
            "type": "table",
            "id": "invites_table",
            "columns": [
                {"id": "mask", "label": "Invite Mask"},
                {"id": "by", "label": "Set by"},
                {"id": "date", "label": "Set date"},
            ],
            "rows": invites_rows,
            "count_label": "invites",
            "empty_text": "No invite exceptions (+I) set in this channel.",
            "actions": [
                {"id": "add", "label": "Add Invite...", "prompt": "Enter invite mask (e.g. nick!*@*):"},
                {"id": "remove", "label": "Remove", "confirm": "Remove selected invite exception(s)?"},
                {"id": "clear", "label": "Clear All", "confirm": "Remove all invite exceptions in this channel?"},
                {"id": "refresh", "label": "Refresh"},
            ],
        }
    ]

    # Tab 5: Topic
    topic_controls = [
        {
            "type": "topic_editor",
            "id": "topic",
            "current": topic_val,
            "current_by": topic_by,
            "current_at": _format_ts(topic_ts) if topic_ts else "",
            "max_len": _DEFAULT_TOPICLEN,
            "history": persisted_topics,
        }
    ]

    # Tab 6: Statistics
    stats_controls = [
        {
            "type": "stats",
            "summary": [
                {"label": "Total users", "value": total_users},
                {"label": "Ops (@)", "value": ops_count},
                {"label": "Halfops (%)", "value": hops_count},
                {"label": "Voices (+)", "value": voices_count},
                {"label": "Regulars", "value": regs_count},
            ],
            "users": user_chips,
        }
    ]

    return {
        "network": network,
        "channel": channel,
        "auto_refresh": True,
        "active_tab": active_tab,
        "tabs": [
            {"id": "modes", "label": "Channel modes"},
            {"id": "bans", "label": f"Bans ({len(bans_rows)})"},
            {"id": "excepts", "label": f"Excepts ({len(excepts_rows)})"},
            {"id": "invites", "label": f"Invites ({len(invites_rows)})"},
            {"id": "topic", "label": "Topic"},
            {"id": "stats", "label": f"Statistics ({total_users})"},
        ],
        "tab_contents": {
            "modes": modes_controls,
            "bans": bans_controls,
            "excepts": excepts_controls,
            "invites": invites_controls,
            "topic": topic_controls,
            "stats": stats_controls,
        },
        "buttons": [
            {"id": "ok", "label": "OK", "primary": True},
            {"id": "apply", "label": "Apply"},
            {"id": "close", "label": "Close"},
        ],
    }


def _open_channel_central(network, channel, ctx):
    global _open_dialog
    cs = ctx.get("channel_state") if ctx else None
    st = _get_chan_state(network, channel, cs)

    _open_dialog["network"] = network
    _open_dialog["channel"] = channel
    _open_dialog["active_tab"] = "modes"

    # Request modes and lists from server
    ferric.raw(f"MODE {channel}")
    ferric.raw(f"MODE {channel} +b")
    ferric.raw(f"MODE {channel} +e")
    ferric.raw(f"MODE {channel} +I")

    spec = build_dialog_spec(network, channel, st, active_tab="modes")
    ferric.dialog_open("channel_central", f"Channel Central: {channel}", spec)


# ---------------------------------------------------------------------------
# Dialog Event Handler
# ---------------------------------------------------------------------------

@ferric.on_dialog("channel_central")
def on_channel_central_dialog(ev, ctx):
    global _open_dialog
    net = _open_dialog.get("network") or (ctx or {}).get("network", "")
    chan = _open_dialog.get("channel") or (ctx or {}).get("chan", "")
    if not chan:
        return

    cs = ctx.get("channel_state") if ctx else None
    st = _get_chan_state(net, chan, cs)

    ev_type = ev.get("type")
    drafts = ev.get("drafts", {})
    cur_tab = ev.get("tab") or _open_dialog.get("active_tab", "modes")
    _open_dialog["active_tab"] = cur_tab

    if ev_type == "tab_change":
        _open_dialog["active_tab"] = ev.get("tab", "modes")

    elif ev_type == "button":
        btn_id = ev.get("button_id")
        if btn_id in ("ok", "apply"):
            # Apply modes
            plus_modes = []
            minus_modes = []
            for flag in _STANDARD_FLAGS:
                if flag in drafts:
                    checked = bool(drafts[flag])
                    if checked and flag not in st["flags"]:
                        plus_modes.append(flag)
                    elif not checked and flag in st["flags"]:
                        minus_modes.append(flag)

            # Key
            k_enable = drafts.get("k_enable")
            k_val = str(drafts.get("k", "")).strip()
            cur_key = st.get("key")
            key_cmd = ""
            if k_enable is not None:
                if k_enable and k_val and k_val != cur_key:
                    key_cmd = f"+k {k_val}"
                elif not k_enable and cur_key:
                    key_cmd = f"-k {cur_key}"

            # Limit
            l_enable = drafts.get("l_enable")
            l_val = str(drafts.get("l", "")).strip()
            cur_limit = st.get("limit")
            limit_cmd = ""
            if l_enable is not None:
                if l_enable and l_val.isdigit() and int(l_val) > 0 and str(l_val) != str(cur_limit or ""):
                    limit_cmd = f"+l {int(l_val)}"
                elif not l_enable and cur_limit:
                    limit_cmd = "-l"

            # Combine flag changes
            mode_str = ""
            if plus_modes:
                mode_str += f"+{''.join(plus_modes)}"
            if minus_modes:
                mode_str += f"-{''.join(minus_modes)}"

            if mode_str:
                ferric.mode(chan, mode_str)
            if key_cmd:
                ferric.mode(chan, key_cmd)
            if limit_cmd:
                ferric.mode(chan, limit_cmd)

            if btn_id == "ok":
                ferric.dialog_close("channel_central")
                _open_dialog["channel"] = ""
            else:
                # Apply: request fresh modes to update
                ferric.raw(f"MODE {chan}")

    elif ev_type == "action":
        action = ev.get("action")
        control = ev.get("control")
        if action == "set_topic" and control == "topic":
            new_topic = drafts.get("topic", "").strip()
            ferric.raw(f"TOPIC {chan} :{new_topic}")
            _record_topic(net, chan, new_topic, ctx.get("me", ""))
            st["topic"] = new_topic
            st["topic_by"] = ctx.get("me", "")
            st["topic_ts"] = int(time.time())
            spec = build_dialog_spec(net, chan, st, active_tab=_open_dialog["active_tab"])
            ferric.dialog_update("channel_central", spec=spec)

        elif action == "clear_topic" and control == "topic":
            ferric.raw(f"TOPIC {chan} :")
            st["topic"] = ""
            st["topic_by"] = ctx.get("me", "")
            st["topic_ts"] = int(time.time())
            spec = build_dialog_spec(net, chan, st, active_tab=_open_dialog["active_tab"])
            ferric.dialog_update("channel_central", spec=spec)

    elif ev_type == "table_action":
        tbl = ev.get("table")
        action = ev.get("action")
        val = ev.get("value")
        selected = ev.get("selected", [])

        mode_char = "b"
        if tbl == "excepts_table":
            mode_char = "e"
        elif tbl == "invites_table":
            mode_char = "I"

        if action == "add" and val:
            ferric.mode(chan, f"+{mode_char} {val.strip()}")
        elif action == "remove" and selected:
            _send_batched_modes(chan, f"-{mode_char}", selected)
        elif action == "clear":
            bucket = st.get(_LIST_MODES.get(mode_char, "bans"), {})
            all_masks = [b["mask"] for b in bucket.values()]
            if all_masks:
                _send_batched_modes(chan, f"-{mode_char}", all_masks)
        elif action == "refresh":
            ferric.raw(f"MODE {chan} +{mode_char}")

    elif ev_type == "refresh":
        spec = build_dialog_spec(net, chan, st, active_tab=_open_dialog["active_tab"])
        ferric.dialog_update("channel_central", spec=spec)

    elif ev_type == "close":
        _open_dialog["channel"] = ""


# ---------------------------------------------------------------------------
# Aliases & Lifecycle
# ---------------------------------------------------------------------------

@ferric.on_load
def on_load():
    _load_topic_hist()
    ferric.echo("Channel Central geladen. /channel oder /cc öffnet die GUI.", target="status")


@ferric.on_unload
def on_unload():
    _save_topic_hist()
    _channels.clear()
    _open_dialog["channel"] = ""


@ferric.alias("channel")
def cmd_channel(args, ctx):
    cmd_cc(args, ctx)


@ferric.alias("cc")
def cmd_cc(args, ctx):
    chan = args[0] if (args and str(args[0]).startswith(("#", "&", "+", "!"))) else None
    if not chan:
        chan = (ctx or {}).get("chan") or (ctx or {}).get("target") or ""
    if not chan or not str(chan).startswith(("#", "&", "+", "!")):
        ferric.echo("Channel Central kann nur in einem Channel verwendet werden.", target="active")
        return

    net = (ctx or {}).get("network") or ""
    _open_channel_central(net, chan, ctx)


@ferric.menu("channel", "Channel Central...")
def menu_open_channel_central(ctx, args, answers):
    chan = (ctx or {}).get("chan") or (ctx or {}).get("target") or ""
    net = (ctx or {}).get("network") or ""
    if chan and str(chan).startswith(("#", "&", "+", "!")):
        _open_channel_central(net, chan, ctx)


# ---------------------------------------------------------------------------
# Live IRC Events & Auto-Close
# ---------------------------------------------------------------------------

@ferric.on("part")
def on_part(ev, ctx):
    net = ev.get("network", "")
    chan = ev.get("channel") or ""
    nick = ev.get("nick") or ""
    me = ev.get("me") or ctx.get("me") or ""
    if chan and nick and me and nick.lower() == me.lower():
        if _open_dialog["channel"].lower() == chan.lower():
            ferric.dialog_close("channel_central")
            _open_dialog["channel"] = ""
        _channels.pop(_key(net, chan), None)


@ferric.on("kick")
def on_kick(ev, ctx):
    net = ev.get("network", "")
    chan = ev.get("channel") or ""
    knick = ev.get("knick") or ""
    me = ev.get("me") or ctx.get("me") or ""
    if chan and knick and me and knick.lower() == me.lower():
        if _open_dialog["channel"].lower() == chan.lower():
            ferric.dialog_close("channel_central")
            _open_dialog["channel"] = ""
        _channels.pop(_key(net, chan), None)


@ferric.on("topic")
def on_topic(ev, ctx):
    net = ev.get("network", "")
    chan = ev.get("channel") or ""
    topic = ev.get("text") or ""
    setter = ev.get("nick") or ""
    if not chan:
        return
    st = _get_chan_state(net, chan)
    st["topic"] = topic
    st["topic_by"] = setter
    st["topic_ts"] = int(time.time())
    _record_topic(net, chan, topic, setter)

    if _open_dialog["channel"].lower() == chan.lower():
        spec = build_dialog_spec(net, chan, st, active_tab=_open_dialog["active_tab"])
        ferric.dialog_update("channel_central", spec=spec)


@ferric.on("mode")
def on_mode(ev, ctx):
    net = ev.get("network", "")
    chan = ev.get("channel") or ""
    raw_modes = ev.get("modes") or ""
    setter = ev.get("nick") or "*"
    if not chan or not raw_modes:
        return

    st = _get_chan_state(net, chan)
    parts = raw_modes.split()
    mode_str = parts[0]
    params = parts[1:]
    p_idx = 0
    adding = True
    now = int(time.time())

    for ch in mode_str:
        if ch == "+":
            adding = True
            continue
        if ch == "-":
            adding = False
            continue

        takes_param = (ch in _PARAM_MODES_ALWAYS) or (ch == "l" and adding)
        param = None
        if takes_param and p_idx < len(params):
            param = params[p_idx]
            p_idx += 1

        if ch in _LIST_MODES and param:
            bucket = st[_LIST_MODES[ch]]
            if adding:
                bucket[param.lower()] = {"mask": param, "by": setter, "ts": now}
            else:
                bucket.pop(param.lower(), None)
        elif ch in ("o", "h", "v", "q", "a") and param:
            user = st["users"].setdefault(param.lower(), {"nick": param, "modes": set(), "prefix_modes": []})
            if adding:
                user.setdefault("modes", set()).add(ch)
            else:
                user.setdefault("modes", set()).discard(ch)
        elif ch == "k":
            st["key"] = param if adding else None
        elif ch == "l":
            st["limit"] = param if adding else None
        elif ch.isalpha():
            if adding:
                st["flags"].add(ch)
            else:
                st["flags"].discard(ch)

    if _open_dialog["channel"].lower() == chan.lower():
        spec = build_dialog_spec(net, chan, st, active_tab=_open_dialog["active_tab"])
        ferric.dialog_update("channel_central", spec=spec)


# ---------------------------------------------------------------------------
# CLI Commands (/ccmodes, /ccban, /ccexcept, /ccinvite, /findban, /ccclear, /cctopic, /cctopics, /ccstats, /ccrefresh)
# ---------------------------------------------------------------------------

@ferric.alias("ccrefresh")
def cmd_ccrefresh(args, ctx):
    chan = args[0] if (args and str(args[0]).startswith(("#", "&", "+", "!"))) else (ctx or {}).get("chan")
    if not chan:
        ferric.echo("Verwendung: /ccrefresh [#channel]", target="active")
        return
    ferric.raw(f"MODE {chan}")
    ferric.raw(f"MODE {chan} +b")
    ferric.raw(f"MODE {chan} +e")
    ferric.raw(f"MODE {chan} +I")
    ferric.echo(f"MODE Abfrage für {chan} gesendet.", target="active")


@ferric.alias("findban")
def cmd_findban(args, ctx):
    chan = (ctx or {}).get("chan") or ""
    net = (ctx or {}).get("network") or ""
    if not chan or not args:
        ferric.echo("Verwendung: /findban <muster>", target="active")
        return
    term = args[0]
    st = _get_chan_state(net, chan)
    pattern = f"*{term.lower()}*"
    hits = [e for e in st["bans"].values() if fnmatch.fnmatchcase(e["mask"].lower(), pattern)]
    if not hits:
        ferric.echo(f"Keine Ban-Treffer für '{term}' in {chan}.", target="active")
        return
    for e in hits:
        ferric.echo(f"Treffer '{term}': {e['mask']} (von {e['by']} am {_format_ts(e['ts'])})", target="active")
