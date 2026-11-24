//! Timers of the script runtime (plan M4): `/timer`, `/timers` and `$timer`.
//!
//! A timer keeps its parsed command, the context it was made in and the clock of
//! that moment. The body reads the context and the clock it was made with, so
//! `$nick` and `$time` keep their creation values. `$!name` reads the live
//! context of the run instead (`Exec::live_value`). The table holds at most
//! `MAX_TIMERS` timers; the runtime runs the due ones in `Runtime::fire_timers`.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::runtime::{wildcard_match, ScriptContext};
use super::text_funcs::CallError;
use super::time_funcs::Clock;
use crate::mrc::Statement;

/// Timers at once (plan section 5).
pub(crate) const MAX_TIMERS: usize = 64;

/// The shortest time between two runs of a timer (plan section 5).
pub(crate) const MIN_PERIOD: Duration = Duration::from_millis(100);

pub(crate) struct Timer {
    /// Lowercase name or number, the key of the table.
    pub(crate) name: String,
    /// The script whose alias or event made the timer. Reloading or unloading it removes the timer.
    pub(crate) owner: String,
    /// The network the timer was made on.
    pub(crate) network: Option<String>,
    /// Online timers stop when their network disconnects; `-o` timers do not.
    pub(crate) online: bool,
    /// Runs left; `None` repeats until it is switched off.
    pub(crate) reps_left: Option<u64>,
    pub(crate) period: Duration,
    pub(crate) next_due: Instant,
    pub(crate) body: Arc<Vec<Statement>>,
    /// The command as written, for `$timer(name).com`.
    pub(crate) command: String,
    pub(crate) context: ScriptContext,
    /// The clock when the timer was made: the time identifiers of the body read it.
    pub(crate) clock: Clock,
    /// Creation order: timers due at the same time run in it.
    pub(crate) seq: u64,
}

#[derive(Default)]
pub(crate) struct TimerTable {
    timers: BTreeMap<String, Timer>,
    next_seq: u64,
    /// The timer whose body runs now (its key), and whether the body has switched it off. A timer
    /// is out of the table while it runs, so the switch-off is recorded here instead.
    running: Option<(String, bool)>,
}

fn key(name: &str) -> String {
    name.to_ascii_lowercase()
}

impl TimerTable {
    pub(crate) fn len(&self) -> usize {
        self.timers.len()
    }

    pub(crate) fn contains(&self, name: &str) -> bool {
        self.timers.contains_key(&key(name))
    }

    pub(crate) fn get(&self, name: &str) -> Option<&Timer> {
        self.timers.get(&key(name))
    }

    /// The timer at position `n` (1 is the first) in the list of timers.
    pub(crate) fn nth(&self, n: usize) -> Option<&Timer> {
        n.checked_sub(1).and_then(|i| self.timers.values().nth(i))
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &Timer> {
        self.timers.values()
    }

    /// Adds a timer. A timer with the same name is replaced; a new name past `MAX_TIMERS` is refused.
    pub(crate) fn insert(&mut self, mut timer: Timer) -> Result<(), String> {
        if !self.contains(&timer.name) && self.timers.len() >= MAX_TIMERS {
            return Err(format!("too many timers: at most {MAX_TIMERS} at once"));
        }
        timer.seq = self.next_seq;
        self.next_seq += 1;
        self.put(timer);
        Ok(())
    }

    /// Puts back a timer taken out for a run. Its place was counted, so the cap is not checked.
    pub(crate) fn put(&mut self, timer: Timer) {
        self.timers.insert(key(&timer.name), timer);
    }

    pub(crate) fn take(&mut self, name: &str) -> Option<Timer> {
        let k = key(name);
        self.cancel_running(|running| running == k);
        self.timers.remove(&k)
    }

    /// Marks the running timer as switched off when `hit` matches its key.
    fn cancel_running(&mut self, hit: impl Fn(&str) -> bool) {
        if let Some((name, cancelled)) = self.running.as_mut() {
            if hit(name) {
                *cancelled = true;
            }
        }
    }

    /// The timer `name` is about to run; a switch-off from its body is recorded until `end_run`.
    pub(crate) fn start_run(&mut self, name: &str) {
        self.running = Some((key(name), false));
    }

    /// Ends the run. True when the body switched the timer off, so it must not be put back.
    pub(crate) fn end_run(&mut self) -> bool {
        self.running.take().is_some_and(|(_, cancelled)| cancelled)
    }

    /// Removes every timer whose name matches a wildcard pattern such as `3?`. Returns the count.
    pub(crate) fn remove_matching(&mut self, pattern: &str) -> usize {
        let before = self.timers.len();
        self.cancel_running(|running| wildcard_match(pattern, running));
        self.timers.retain(|name, _| !wildcard_match(pattern, name));
        before - self.timers.len()
    }

    pub(crate) fn clear(&mut self) {
        self.cancel_running(|_| true);
        self.timers.clear();
    }

    /// Removes the online timers of a network that disconnected.
    pub(crate) fn remove_network(&mut self, network: &str) {
        self.timers
            .retain(|_, t| !(t.online && t.network.as_deref() == Some(network)));
    }

    /// Removes the timers a script made, when the script is reloaded or unloaded.
    pub(crate) fn remove_owner(&mut self, owner: &str) {
        // Owners are stored in lowercase, as script names are case-insensitive.
        let owner = owner.to_ascii_lowercase();
        self.timers.retain(|_, t| t.owner != owner);
    }

    /// Keys of the timers due at `now`, earliest first.
    pub(crate) fn due(&self, now: Instant) -> Vec<String> {
        let mut due: Vec<(Instant, u64, String)> = self
            .timers
            .iter()
            .filter(|(_, t)| t.next_due <= now)
            .map(|(k, t)| (t.next_due, t.seq, k.clone()))
            .collect();
        due.sort();
        due.into_iter().map(|(_, _, k)| k).collect()
    }

    /// The lowest timer number from 1 that no timer has; an unnamed `/timer` takes it.
    pub(crate) fn first_free_id(&self) -> Option<u64> {
        (1..=MAX_TIMERS as u64).find(|n| !self.contains(&n.to_string()))
    }
}

/// `$timer(...)`: the number of timers for `0`, the name of the timer at a position or with a
/// name, and `$null` when there is none. Properties: `com`, `reps`, `secs`, `type`.
pub(crate) fn ident(
    table: &TimerTable,
    now: Instant,
    args: &[String],
    prop: Option<&str>,
) -> Result<String, CallError> {
    let arg = args.first().map_or("", |s| s.trim());
    if arg.is_empty() {
        return Err(CallError::Arguments("$timer needs a timer number or name".into()));
    }
    if arg == "0" && prop.is_none() {
        return Ok(table.len().to_string());
    }
    let found = match arg.parse::<usize>() {
        Ok(n) => table.nth(n),
        Err(_) => table.get(arg),
    };
    let Some(timer) = found else {
        return Ok(String::new());
    };
    Ok(match prop {
        None => timer.name.clone(),
        Some("com") => timer.command.clone(),
        Some("reps") => timer.reps_left.unwrap_or(0).to_string(),
        Some("secs") => timer
            .next_due
            .saturating_duration_since(now)
            .as_millis()
            .div_ceil(1000)
            .to_string(),
        Some("type") => if timer.online { "online" } else { "offline" }.to_string(),
        Some(other) => {
            return Err(CallError::Arguments(format!("$timer has no property .{other}")));
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timer(name: &str, owner: &str, network: Option<&str>, online: bool, due: Instant) -> Timer {
        Timer {
            name: key(name),
            owner: owner.into(),
            network: network.map(str::to_string),
            online,
            reps_left: Some(1),
            period: MIN_PERIOD,
            next_due: due,
            body: Arc::new(Vec::new()),
            command: "echo x".into(),
            context: ScriptContext::default(),
            clock: Clock::now(),
            seq: 0,
        }
    }

    #[test]
    fn the_cap_refuses_a_new_name_but_not_a_replacement() {
        let now = Instant::now();
        let mut table = TimerTable::default();
        for i in 0..MAX_TIMERS {
            table.insert(timer(&i.to_string(), "s", None, true, now)).unwrap();
        }
        assert!(table.insert(timer("extra", "s", None, true, now)).is_err());
        assert!(table.insert(timer("3", "s", None, true, now)).is_ok());
        assert_eq!(table.len(), MAX_TIMERS);
    }

    #[test]
    fn network_and_owner_removal_respect_the_switches() {
        let now = Instant::now();
        let mut table = TimerTable::default();
        table.insert(timer("on", "s", Some("net"), true, now)).unwrap();
        table.insert(timer("off", "s", Some("net"), false, now)).unwrap();
        table.insert(timer("other", "t", Some("net2"), true, now)).unwrap();
        table.remove_network("net");
        assert!(!table.contains("on"));
        assert!(table.contains("off"), "-o timers survive a disconnect");
        table.remove_owner("t");
        assert!(!table.contains("other"));
    }

    #[test]
    fn timers_due_at_the_same_time_run_in_creation_order() {
        let now = Instant::now();
        let mut table = TimerTable::default();
        table.insert(timer("b", "s", None, true, now)).unwrap();
        table.insert(timer("a", "s", None, true, now)).unwrap();
        assert_eq!(table.due(now), ["b", "a"]);
    }

    #[test]
    fn due_timers_come_earliest_first() {
        let start = Instant::now();
        let mut table = TimerTable::default();
        table.insert(timer("b", "s", None, true, start + Duration::from_secs(2))).unwrap();
        table.insert(timer("a", "s", None, true, start + Duration::from_secs(1))).unwrap();
        assert_eq!(table.due(start + Duration::from_secs(5)), ["a", "b"]);
        assert_eq!(table.due(start + Duration::from_secs(1)), ["a"]);
    }

    #[test]
    fn identifiers_read_count_position_name_and_properties() {
        let now = Instant::now();
        let mut table = TimerTable::default();
        table.insert(timer("t1", "s", None, false, now + Duration::from_millis(2500))).unwrap();
        let arg = |s: &str| vec![s.to_string()];
        assert_eq!(ident(&table, now, &arg("0"), None).unwrap(), "1");
        assert_eq!(ident(&table, now, &arg("1"), None).unwrap(), "t1");
        assert_eq!(ident(&table, now, &arg("T1"), None).unwrap(), "t1");
        assert_eq!(ident(&table, now, &arg("nope"), None).unwrap(), "");
        assert_eq!(ident(&table, now, &arg("t1"), Some("type")).unwrap(), "offline");
        assert_eq!(ident(&table, now, &arg("t1"), Some("com")).unwrap(), "echo x");
        assert_eq!(ident(&table, now, &arg("t1"), Some("secs")).unwrap(), "3");
        assert!(ident(&table, now, &arg("t1"), Some("bogus")).is_err());
    }
}
