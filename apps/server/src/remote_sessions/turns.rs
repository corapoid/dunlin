//! The live actor owns this clock. SQL and incoming receipts cannot restart it.
use super::*;

struct Active {
    command: wire::TurnCommand,
    deadline: Instant,
}
#[derive(Default)]
pub(super) struct Clock {
    active: Option<Active>,
    last_index: u64,
}
impl Clock {
    pub(super) fn scope(&self, now: Instant) -> crate::store::Result<(String, Instant)> {
        self.check(now)?;
        let active = self.active.as_ref().ok_or_else(denied)?;
        Ok((active.command.identity.turn_id.clone(), active.deadline))
    }
    pub(super) fn matches_scope(
        &self,
        scope: &wire::broker::BrokerScope,
        now: Instant,
    ) -> crate::store::Result<()> {
        self.check(now)?;
        if let wire::broker::BrokerScope::Turn { turn } = scope {
            let active = self.active.as_ref().ok_or_else(denied)?;
            if active.command.session != turn.session
                || active.command.identity.turn_id != turn.turn_id
                || active.command.turn_index != turn.turn_index
            {
                return Err(denied());
            }
        }
        Ok(())
    }
    pub(super) fn check(&self, now: Instant) -> crate::store::Result<()> {
        if self
            .active
            .as_ref()
            .is_some_and(|turn| now >= turn.deadline)
        {
            return Err(denied());
        }
        Ok(())
    }
    pub(super) fn begin(
        &mut self,
        command: wire::TurnCommand,
        session_deadline: Instant,
        now: Instant,
    ) -> crate::store::Result<()> {
        command.validate().map_err(|_| denied())?;
        if self.active.is_some()
            || command.turn_index <= self.last_index
            || now >= session_deadline
            || wire::session_turn_fingerprint(&command).map_err(|_| denied())?
                != command.identity.fingerprint
        {
            return Err(denied());
        }
        self.last_index = command.turn_index;
        self.active = Some(Active {
            deadline: session_deadline.min(now + Duration::from_secs(command.budgets.max_seconds)),
            command,
        });
        Ok(())
    }
    /// Only the actor's committed SDK activity read may finish a turn. An
    /// accepted/replayed wire receipt never clears or replaces the clock.
    pub(super) fn reconcile(&mut self, active: Option<&str>) -> crate::store::Result<()> {
        match (self.active.as_ref(), active) {
            (Some(turn), Some(id)) if turn.command.identity.turn_id == id => Ok(()),
            (_, None) => {
                self.active = None;
                Ok(())
            }
            _ => Err(denied()),
        }
    }
    pub(super) fn submit_deadline(
        &self,
        command: &wire::TurnCommand,
        now: Instant,
    ) -> crate::store::Result<Instant> {
        self.check(now)?;
        let turn = self.active.as_ref().ok_or_else(denied)?;
        if &turn.command != command {
            return Err(denied());
        }
        Ok(turn.deadline)
    }
}

#[cfg(test)]
mod tests;
