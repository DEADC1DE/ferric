// Parser for /server. Syntax for the options Ferric supports:
//   /server [-46emp...] [host[:port[:password]]] [port] [password] [-i nick altnick email realname]
//   /server            reconnects to the last server used
//   /server N          connects to entry N of the server list
// Ferric does not implement -n, -d, -t (STARTTLS) and the server-group form, so these are refused
// with a message instead of connecting in a different way than the user asked.

const DEFAULT_PORT = 6667;
const DEFAULT_TLS_PORT = 6697;
const PORT_TOKEN = /^([+*]?)(\d{1,5})$/;
const SWITCH_TOKEN = /^-[46demntpfoczu]+$/;

// Switches that are read but have no effect in Ferric: they change window or perform behaviour
// that Ferric does not have.
const IGNORED_SWITCHES = new Set(["4", "6", "m", "p", "f", "o", "c", "z", "u"]);

export function parseServerArgs(args) {
  let tls = false;
  let identity = null;
  const positional = [];

  for (let i = 0; i < args.length; i += 1) {
    const token = args[i];
    if (token === "-i") {
      // -i takes the rest of the line: nick, alternative nick, e-mail, then the real name.
      const [nick, altnick, email, ...nameWords] = args.slice(i + 1);
      identity = {
        nick: nick || null,
        altnick: altnick || null,
        email: email || null,
        realname: nameWords.join(" ") || null,
      };
      break;
    }
    if (SWITCH_TOKEN.test(token)) {
      for (const letter of token.slice(1)) {
        if (letter === "n") return refuse("-n (create the window without connecting)");
        if (letter === "d") return refuse("-d (set the details without connecting)");
        if (letter === "t") return refuse("-t (STARTTLS)");
        if (letter === "e") tls = true;
        else if (!IGNORED_SWITCHES.has(letter)) return refuse(`-${letter}`);
      }
      continue;
    }
    if (token.startsWith("-")) return refuse(token);
    positional.push(token);
  }

  if (positional.length === 0) {
    return tls ? { ok: true, action: "last", tls: true, identity } : { ok: true, action: "last", identity };
  }
  if (positional.length === 1 && /^\d+$/.test(positional[0])) {
    return tls
      ? { ok: true, action: "list", index: Number(positional[0]), tls: true, identity }
      : { ok: true, action: "list", index: Number(positional[0]), identity };
  }

  // host, optionally followed by :port and :password in one word.
  if (positional[0].startsWith("::")) {
    return { ok: false, error: "IPv6 addresses are not supported in /server yet." };
  }
  const pieces = positional[0].split(":");
  if (pieces.length > 3) return { ok: false, error: "IPv6 addresses are not supported in /server yet." };
  const host = pieces[0];
  let portToken = pieces[1] || null;
  let password = pieces[2] || null;
  let next = 1;
  if (portToken === null && positional[1] !== undefined && PORT_TOKEN.test(positional[1])) {
    portToken = positional[1];
    next = 2;
  }
  if (password === null && positional[next] !== undefined) {
    password = positional[next];
    next += 1;
  }
  if (positional.length > next) {
    return { ok: false, error: `Too many arguments for /server: ${positional.slice(next).join(" ")}` };
  }
  if (!host) return { ok: false, error: "Use /server <host> [port] [password]." };

  let port = null;
  if (portToken !== null) {
    const match = PORT_TOKEN.exec(portToken);
    if (!match) return { ok: false, error: `Not a port: ${portToken}` };
    if (match[1] === "*") return refuse("the * port prefix (STARTTLS)");
    if (match[1] === "+") tls = true;
    port = Number(match[2]);
    if (port < 1 || port > 65535) return { ok: false, error: `Port out of range: ${port}` };
  }

  return {
    ok: true,
    action: "connect",
    host,
    port: port ?? (tls ? DEFAULT_TLS_PORT : DEFAULT_PORT),
    tls,
    password: password || null,
    identity,
  };
}

function refuse(what) {
  return { ok: false, error: `/server ${what} is not supported in Ferric yet.` };
}

// Connection settings for one server. -i sets the nick and the ident (the part of the e-mail
// before @); without it the nick is used for everything.
export function buildServerTarget(parsed, defaultNick) {
  return applyServerOverrides({
    host: parsed.host,
    port: parsed.port,
    tls: parsed.tls,
    password: parsed.password,
    nick: defaultNick,
    altnick: `${defaultNick}_`,
    username: defaultNick,
    realname: defaultNick,
  }, parsed);
}

// Applies -e (TLS) and -i (identity) overrides from parsed /server options onto a base target.
export function applyServerOverrides(base, parsed) {
  const identity = parsed.identity || {};
  const nick = identity.nick || base.nick;
  const username = identity.email ? identity.email.split("@")[0] || nick : base.username;
  return {
    ...base,
    tls: parsed.tls ? true : base.tls,
    nick,
    altnick: identity.altnick || (identity.nick ? `${nick}_` : base.altnick),
    username,
    realname: identity.realname || (identity.nick ? nick : base.realname),
  };
}
