//! The authenticated systemd user-manager transport behind `launch_txn::UserManagerPort`.
//!
//! It sends the one fixed `StartTransientUnit` request and observes the unique unit through the
//! manager's documented interfaces. It deliberately cannot prove that a request will never
//! execute: an absent unit, an empty job queue or elapsed time never establishes that, so
//! `proves_never_executable` stays false and a lost acknowledgement remains ambiguous.

use super::{
    launch_txn::{SendError, UnitState, UserManagerPort},
    user_launcher::{START_SIGNATURE, UserLaunchRequest},
    user_manager::VerifiedUserManager,
};

/// Outcome of a manager call that did not return a normal reply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BusFault {
    /// Revalidation failed before any byte was written: nothing was sent.
    NotSent,
    /// The authenticated manager itself answered with this D-Bus error.
    Remote(String),
    /// Transport failure, timeout or an unattributable reply: the request may have been acted on.
    Uncertain,
}
use crate::integrations::dbus::{Value, Writer};

const MANAGER_PATH: &str = "/org/freedesktop/systemd1";
const MANAGER_INTERFACE: &str = "org.freedesktop.systemd1.Manager";
const UNIT_INTERFACE: &str = "org.freedesktop.systemd1.Unit";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
const NO_SUCH_UNIT: &str = "org.freedesktop.systemd1.NoSuchUnit";

/// Errors the manager returns before it queues anything. Anything else (notably `UnitExists`,
/// which may be our own earlier request) leaves the outcome uncertain.
const NONEXECUTING_ERRORS: [&str; 3] = [
    "org.freedesktop.DBus.Error.InvalidArgs",
    "org.freedesktop.DBus.Error.AccessDenied",
    "org.freedesktop.DBus.Error.NotSupported",
];

/// A bounded, sender-pinned call channel to the authenticated manager.
pub trait ManagerBus {
    fn call(
        &mut self,
        path: &str,
        interface: &str,
        member: &str,
        signature: &str,
        body: &[u8],
    ) -> Result<Vec<Value>, BusFault>;
}

impl ManagerBus for VerifiedUserManager {
    fn call(
        &mut self,
        path: &str,
        interface: &str,
        member: &str,
        signature: &str,
        body: &[u8],
    ) -> Result<Vec<Value>, BusFault> {
        self.call_manager(path, interface, member, signature, body)
    }
}

pub struct DbusUserManagerPort<B: ManagerBus> {
    bus: B,
}

impl<B: ManagerBus> DbusUserManagerPort<B> {
    pub fn new(bus: B) -> Self {
        Self { bus }
    }

    fn property(&mut self, unit_path: &str, name: &str) -> anyhow::Result<Value> {
        let mut body = Writer::default();
        body.string(UNIT_INTERFACE);
        body.string(name);
        let values = self
            .bus
            .call(unit_path, PROPERTIES, "Get", "ss", &body.buf)
            .map_err(|fault| anyhow::anyhow!("unit property query failed: {fault:?}"))?;
        match values.into_iter().next() {
            Some(Value::Variant(inner)) => Ok(*inner),
            _ => anyhow::bail!("unexpected unit property reply"),
        }
    }
}

fn valid_unit_name(name: &str) -> bool {
    name.len() <= 255
        && name.ends_with(".service")
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_'))
}

impl<B: ManagerBus> UserManagerPort for DbusUserManagerPort<B> {
    fn send_start(&mut self, request: &UserLaunchRequest) -> Result<(), SendError> {
        match self.bus.call(
            MANAGER_PATH,
            MANAGER_INTERFACE,
            "StartTransientUnit",
            START_SIGNATURE,
            &request.body,
        ) {
            Ok(values) => match values.as_slice() {
                [Value::Path(_)] => Ok(()),
                // The manager replied but not with a job: treat the effect as unknown.
                _ => Err(SendError::AckLost),
            },
            // Revalidation failed before any byte was written.
            Err(BusFault::NotSent) => Err(SendError::Rejected),
            Err(BusFault::Remote(error)) if NONEXECUTING_ERRORS.contains(&error.as_str()) => {
                Err(SendError::Rejected)
            }
            Err(BusFault::Remote(_) | BusFault::Uncertain) => Err(SendError::AckLost),
        }
    }

    fn unit_state(&mut self, unit_name: &str) -> anyhow::Result<UnitState> {
        anyhow::ensure!(valid_unit_name(unit_name), "invalid launch unit name");
        let mut body = Writer::default();
        body.string(unit_name);
        let values = match self
            .bus
            .call(MANAGER_PATH, MANAGER_INTERFACE, "GetUnit", "s", &body.buf)
        {
            Ok(values) => values,
            Err(BusFault::Remote(error)) if error == NO_SUCH_UNIT => {
                return Ok(UnitState::Absent);
            }
            Err(fault) => anyhow::bail!("unit lookup failed: {fault:?}"),
        };
        let [Value::Path(unit_path)] = values.as_slice() else {
            anyhow::bail!("unexpected unit lookup reply");
        };
        let Value::Str(state) = self.property(unit_path, "ActiveState")? else {
            anyhow::bail!("unexpected active state type");
        };
        match state.as_str() {
            "active" | "activating" | "reloading" | "deactivating" | "refreshing" => {
                Ok(UnitState::Active)
            }
            "inactive" | "failed" => {
                // A unit with a queued job (a first start or a restart) is not finished.
                let Value::Struct(job) = self.property(unit_path, "Job")? else {
                    anyhow::bail!("unexpected unit job type");
                };
                match job.first() {
                    Some(Value::U32(0)) => Ok(UnitState::Finished),
                    Some(Value::U32(_)) => Ok(UnitState::Active),
                    _ => anyhow::bail!("unexpected unit job shape"),
                }
            }
            other => anyhow::bail!("unknown unit state {other}"),
        }
    }

    /// Never true here. The manager exposes no durable record that a request was fenced, and a
    /// missing unit, an empty queue or a completed (collected) unit do not show non-execution.
    fn proves_never_executable(&mut self, _unit_name: &str) -> anyhow::Result<bool> {
        Ok(false)
    }
}

/// A manager whose attestation can expire. The production `VerifiedUserManager` is only trusted
/// for a few seconds after it was independently authenticated.
pub trait Freshness {
    fn is_fresh(&self) -> bool;
}

impl Freshness for VerifiedUserManager {
    fn is_fresh(&self) -> bool {
        VerifiedUserManager::is_fresh(self)
    }
}

/// Re-authenticates the manager from scratch whenever the previous attestation has expired, so a
/// long benchmark can keep querying without ever relaxing the freshness rule. If a fresh
/// authentication cannot be obtained, nothing is sent: the call fails as `NotSent`.
pub struct ReauthenticatingBus<M, F> {
    current: Option<M>,
    authenticate: F,
}

impl<M, F> ReauthenticatingBus<M, F>
where
    M: ManagerBus + Freshness,
    F: FnMut() -> anyhow::Result<M>,
{
    pub fn new(authenticate: F) -> Self {
        Self {
            current: None,
            authenticate,
        }
    }
}

impl<M, F> ManagerBus for ReauthenticatingBus<M, F>
where
    M: ManagerBus + Freshness,
    F: FnMut() -> anyhow::Result<M>,
{
    fn call(
        &mut self,
        path: &str,
        interface: &str,
        member: &str,
        signature: &str,
        body: &[u8],
    ) -> Result<Vec<Value>, BusFault> {
        if !self.current.as_ref().is_some_and(Freshness::is_fresh) {
            // Drop the stale manager before authenticating a new one; never reuse it.
            self.current = None;
            self.current = Some((self.authenticate)().map_err(|_| BusFault::NotSent)?);
        }
        let manager = self.current.as_mut().ok_or(BusFault::NotSent)?;
        manager.call(path, interface, member, signature, body)
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::VecDeque};

    use super::*;
    use crate::autotune::research::launch_txn::{LaunchOutcome, run};

    struct FakeBus {
        replies: VecDeque<Result<Vec<Value>, BusFault>>,
        calls: Vec<(String, String, String)>,
    }

    impl FakeBus {
        fn new(replies: Vec<Result<Vec<Value>, BusFault>>) -> Self {
            Self {
                replies: replies.into(),
                calls: Vec::new(),
            }
        }
    }

    impl ManagerBus for FakeBus {
        fn call(
            &mut self,
            path: &str,
            interface: &str,
            member: &str,
            _: &str,
            _: &[u8],
        ) -> Result<Vec<Value>, BusFault> {
            self.calls
                .push((path.into(), interface.into(), member.into()));
            self.replies.pop_front().unwrap_or(Err(BusFault::Uncertain))
        }
    }

    fn request() -> UserLaunchRequest {
        UserLaunchRequest {
            unit_name: "stutter-research-0123456789abcdef-0000.service".into(),
            body: vec![1, 2, 3],
        }
    }

    fn job() -> Result<Vec<Value>, BusFault> {
        Ok(vec![Value::Path("/org/freedesktop/systemd1/job/1".into())])
    }

    fn start(reply: Result<Vec<Value>, BusFault>) -> Result<(), SendError> {
        DbusUserManagerPort::new(FakeBus::new(vec![reply])).send_start(&request())
    }

    #[test]
    fn the_fixed_request_is_sent_once_to_the_manager_start_method() {
        let mut port = DbusUserManagerPort::new(FakeBus::new(vec![job()]));
        assert!(port.send_start(&request()).is_ok());
        assert_eq!(
            port.bus.calls,
            [(
                MANAGER_PATH.to_owned(),
                MANAGER_INTERFACE.to_owned(),
                "StartTransientUnit".to_owned()
            )]
        );
    }

    #[test]
    fn only_nonexecuting_failures_are_rejections_everything_else_is_a_lost_ack() {
        let remote = |e: &str| Err(BusFault::Remote(e.into()));
        assert!(matches!(
            start(Err(BusFault::NotSent)),
            Err(SendError::Rejected)
        ));
        assert!(matches!(
            start(remote("org.freedesktop.DBus.Error.InvalidArgs")),
            Err(SendError::Rejected)
        ));
        assert!(matches!(
            start(remote("org.freedesktop.DBus.Error.AccessDenied")),
            Err(SendError::Rejected)
        ));
        // The unit may be our own earlier request.
        assert!(matches!(
            start(remote("org.freedesktop.systemd1.UnitExists")),
            Err(SendError::AckLost)
        ));
        assert!(matches!(
            start(Err(BusFault::Uncertain)),
            Err(SendError::AckLost)
        ));
        assert!(matches!(
            start(Ok(vec![Value::Str("not a job".into())])),
            Err(SendError::AckLost)
        ));
    }

    fn state_of(replies: Vec<Result<Vec<Value>, BusFault>>) -> anyhow::Result<UnitState> {
        DbusUserManagerPort::new(FakeBus::new(replies)).unit_state(&request().unit_name)
    }

    fn unit_path() -> Result<Vec<Value>, BusFault> {
        Ok(vec![Value::Path("/org/freedesktop/systemd1/unit/u".into())])
    }

    fn active_state(state: &str) -> Result<Vec<Value>, BusFault> {
        Ok(vec![Value::Variant(Box::new(Value::Str(state.into())))])
    }

    fn pending_job(id: u32) -> Result<Vec<Value>, BusFault> {
        Ok(vec![Value::Variant(Box::new(Value::Struct(vec![
            Value::U32(id),
            Value::Path("/".into()),
        ])))])
    }

    #[test]
    fn unit_states_map_conservatively() {
        assert_eq!(
            state_of(vec![Err(BusFault::Remote(NO_SUCH_UNIT.into()))]).unwrap(),
            UnitState::Absent
        );
        for live in ["active", "activating", "deactivating", "reloading"] {
            assert_eq!(
                state_of(vec![unit_path(), active_state(live)]).unwrap(),
                UnitState::Active
            );
        }
        assert_eq!(
            state_of(vec![unit_path(), active_state("failed"), pending_job(0)]).unwrap(),
            UnitState::Finished
        );
        // A failed unit with a queued (restart) job is not finished.
        assert_eq!(
            state_of(vec![unit_path(), active_state("failed"), pending_job(3)]).unwrap(),
            UnitState::Active
        );
        // Inactive with a queued job has not started; with no job it is finished.
        assert_eq!(
            state_of(vec![unit_path(), active_state("inactive"), pending_job(7)]).unwrap(),
            UnitState::Active
        );
        assert_eq!(
            state_of(vec![unit_path(), active_state("inactive"), pending_job(0)]).unwrap(),
            UnitState::Finished
        );
        assert!(state_of(vec![unit_path(), active_state("mystery")]).is_err());
        assert!(state_of(vec![Err(BusFault::Uncertain)]).is_err());
        assert!(state_of(vec![Err(BusFault::Remote("other".into()))]).is_err());
    }

    #[test]
    fn a_hostile_unit_name_is_refused_before_any_call() {
        let mut port = DbusUserManagerPort::new(FakeBus::new(Vec::new()));
        assert!(port.unit_state("../evil.service").is_err());
        assert!(port.unit_state("a b.service").is_err());
        assert!(port.unit_state("noservice").is_err());
        assert!(port.bus.calls.is_empty());
    }

    #[test]
    fn a_lost_ack_with_no_unit_stays_ambiguous_through_the_transaction() {
        // Lost start reply, then three absent snapshots: no proof of non-execution exists.
        let absent = || Err(BusFault::Remote(NO_SUCH_UNIT.into()));
        let mut port = DbusUserManagerPort::new(FakeBus::new(vec![
            Err(BusFault::Uncertain),
            absent(),
            absent(),
            absent(),
        ]));
        let outcome = run(&mut port, &request(), &mut |_| Ok(())).unwrap();
        assert!(matches!(outcome, LaunchOutcome::Ambiguous(_)));
        // Exactly one start call was ever made.
        assert_eq!(
            port.bus
                .calls
                .iter()
                .filter(|(_, _, m)| m == "StartTransientUnit")
                .count(),
            1
        );
    }

    #[test]
    fn a_late_unit_after_a_lost_ack_is_observed_not_resent() {
        let absent = || Err(BusFault::Remote(NO_SUCH_UNIT.into()));
        let mut port = DbusUserManagerPort::new(FakeBus::new(vec![
            Err(BusFault::Uncertain),
            absent(),
            unit_path(),
            active_state("active"),
        ]));
        let outcome = run(&mut port, &request(), &mut |_| Ok(())).unwrap();
        assert_eq!(outcome, LaunchOutcome::Running);
    }

    struct Aging {
        id: u32,
        fresh: std::rc::Rc<std::cell::Cell<bool>>,
        log: std::rc::Rc<RefCell<Vec<u32>>>,
    }

    impl ManagerBus for Aging {
        fn call(
            &mut self,
            _: &str,
            _: &str,
            _: &str,
            _: &str,
            _: &[u8],
        ) -> Result<Vec<Value>, BusFault> {
            self.log.borrow_mut().push(self.id);
            Ok(Vec::new())
        }
    }

    impl Freshness for Aging {
        fn is_fresh(&self) -> bool {
            self.fresh.get()
        }
    }

    #[test]
    fn an_expired_manager_is_replaced_by_a_fresh_authentication_before_every_call() {
        let log = std::rc::Rc::new(RefCell::new(Vec::new()));
        let mut next = 0;
        let l = log.clone();
        let mut bus = ReauthenticatingBus::new(move || {
            next += 1;
            Ok(Aging {
                id: next,
                // Each authentication yields its own manager, fresh until it is expired.
                fresh: std::rc::Rc::new(std::cell::Cell::new(true)),
                log: l.clone(),
            })
        });
        // The first call authenticates, and a still-fresh manager is reused.
        bus.call("/", "i", "m", "", &[]).unwrap();
        bus.call("/", "i", "m", "", &[]).unwrap();
        assert_eq!(*log.borrow(), [1, 1]);
        // Once the attestation expires the old manager is never used again.
        bus.current.as_ref().unwrap().fresh.set(false);
        bus.call("/", "i", "m", "", &[]).unwrap();
        assert_eq!(*log.borrow(), [1, 1, 2]);
    }

    #[test]
    fn a_failed_reauthentication_sends_nothing() {
        let mut bus: ReauthenticatingBus<Aging, _> =
            ReauthenticatingBus::new(|| Err(anyhow::anyhow!("session changed")));
        assert_eq!(bus.call("/", "i", "m", "", &[]), Err(BusFault::NotSent));
    }
}
