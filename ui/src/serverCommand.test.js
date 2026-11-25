import { test } from "node:test";
import assert from "node:assert/strict";
import { applyServerOverrides, buildServerTarget, parseServerArgs } from "./serverCommand.js";

test("no arguments reconnects to the last server", () => {
  assert.deepEqual(parseServerArgs([]).action, "last");
});

test("a single number selects a server-list entry", () => {
  assert.deepEqual(parseServerArgs(["3"]), { ok: true, action: "list", index: 3, identity: null });
});

test("host, port and password as separate words", () => {
  const r = parseServerArgs(["127.0.0.1", "6667", "testpw"]);
  assert.equal(r.ok, true);
  assert.equal(r.host, "127.0.0.1");
  assert.equal(r.port, 6667);
  assert.equal(r.tls, false);
  assert.equal(r.password, "testpw");
});

test("host:port:password in one word", () => {
  const r = parseServerArgs(["127.0.0.1:6667:testpw"]);
  assert.equal(r.port, 6667);
  assert.equal(r.password, "testpw");
});

test("a plus before the port selects TLS", () => {
  const r = parseServerArgs(["irc.example.org", "+6697"]);
  assert.equal(r.tls, true);
  assert.equal(r.port, 6697);
});

test("the -e switch selects TLS and the default TLS port", () => {
  const r = parseServerArgs(["-e", "irc.example.org"]);
  assert.equal(r.tls, true);
  assert.equal(r.port, 6697);
});

test("without a port the plain default is 6667", () => {
  assert.equal(parseServerArgs(["irc.example.org"]).port, 6667);
});

test("a password without a port is not taken as a port", () => {
  const r = parseServerArgs(["irc.example.org", "secret"]);
  assert.equal(r.port, 6667);
  assert.equal(r.password, "secret");
});

test("-i takes nick, alternative nick, e-mail and the rest as real name", () => {
  const r = parseServerArgs(["127.0.0.1", "6667", "pw", "-i", "tnick", "tnick2", "tmail@example.org", "Real", "Name"]);
  assert.deepEqual(r.identity, { nick: "tnick", altnick: "tnick2", email: "tmail@example.org", realname: "Real Name" });
  const target = buildServerTarget(r, "fallback");
  assert.equal(target.nick, "tnick");
  assert.equal(target.username, "tmail");
  assert.equal(target.realname, "Real Name");
  assert.equal(target.altnick, "tnick2");
});

test("without -i the default nick is used for nick, ident and real name", () => {
  const target = buildServerTarget(parseServerArgs(["irc.example.org"]), "ferric");
  assert.deepEqual(
    { nick: target.nick, username: target.username, realname: target.realname, altnick: target.altnick },
    { nick: "ferric", username: "ferric", realname: "ferric", altnick: "ferric_" },
  );
});

test("-n, -d and -t are refused, not run", () => {
  for (const flag of ["-n", "-d", "-t", "-dn"]) {
    const r = parseServerArgs([flag, "irc.example.org"]);
    assert.equal(r.ok, false, flag);
    assert.match(r.error, /not supported/);
  }
  assert.equal(parseServerArgs(["irc.example.org", "*6697"]).ok, false);
});

test("unknown switches are refused", () => {
  assert.equal(parseServerArgs(["-zz9", "irc.example.org"]).ok, false);
});

test("out-of-range and extra arguments are errors", () => {
  assert.equal(parseServerArgs(["irc.example.org", "70000"]).ok, false);
  assert.equal(parseServerArgs(["irc.example.org", "6667", "pw", "extra"]).ok, false);
});

test("trailing colon uses default port and ::1 reports IPv6 unsupported", () => {
  const trailing = parseServerArgs(["irc.example.org:"]);
  assert.equal(trailing.ok, true);
  assert.equal(trailing.port, 6667);

  const skippedPort = parseServerArgs(["irc.example.org::secret"]);
  assert.equal(skippedPort.ok, true);
  assert.equal(skippedPort.port, 6667);
  assert.equal(skippedPort.password, "secret");

  const v6 = parseServerArgs(["::1"]);
  assert.equal(v6.ok, false);
  assert.match(v6.error, /IPv6/);
});

test("-e and -i are kept on list and last actions", () => {
  const list = parseServerArgs(["-e", "2", "-i", "n1", "n2", "u@h", "Full Name"]);
  assert.equal(list.ok, true);
  assert.equal(list.action, "list");
  assert.equal(list.index, 2);
  assert.equal(list.tls, true);
  assert.equal(list.identity.nick, "n1");

  const last = parseServerArgs(["-e", "-i", "n1"]);
  assert.equal(last.ok, true);
  assert.equal(last.action, "last");
  assert.equal(last.tls, true);
  assert.equal(last.identity.nick, "n1");

  const merged = applyServerOverrides(
    { host: "irc.example.org", port: 6667, tls: false, password: null, nick: "old", altnick: "old_", username: "u", realname: "Old" },
    list,
  );
  assert.equal(merged.tls, true);
  assert.equal(merged.nick, "n1");
  assert.equal(merged.altnick, "n2");
  assert.equal(merged.username, "u");
  assert.equal(merged.realname, "Full Name");
});
