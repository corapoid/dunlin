//! Compact exact RPC identities exclude prompt/profile contents.
use super::*;

impl SessionControllerMessage {
    pub fn channel(&self) -> &ChannelFence {
        match self {
            Self::OpenSession { command } => &command.channel,
            Self::SubmitTurn { command } => &command.session.channel,
            Self::RenewSession { session, .. } | Self::CloseSession { session } => &session.channel,
            Self::RenewTurn { turn, .. } | Self::CancelTurn { turn } => &turn.session.channel,
            Self::ReconcileSession { channel, .. }
            | Self::ReconcileTurn { channel, .. }
            | Self::ReplayStream { channel, .. }
            | Self::AckStream { channel, .. } => channel,
        }
    }
    pub fn reference(&self) -> SessionRequestRef {
        match self {
            Self::OpenSession { command } => SessionRequestRef::Open {
                identity: command.identity.clone(),
            },
            Self::SubmitTurn { command } => SessionRequestRef::Submit {
                session: command.session.clone(),
                identity: command.identity.clone(),
            },
            Self::RenewSession {
                session,
                sequence,
                lease_duration_ms,
            } => SessionRequestRef::RenewSession {
                session: session.clone(),
                sequence: *sequence,
                lease_duration_ms: *lease_duration_ms,
            },
            Self::RenewTurn {
                turn,
                sequence,
                lease_duration_ms,
            } => SessionRequestRef::RenewTurn {
                turn: turn.clone(),
                sequence: *sequence,
                lease_duration_ms: *lease_duration_ms,
            },
            Self::CancelTurn { turn } => SessionRequestRef::Cancel { turn: turn.clone() },
            Self::CloseSession { session } => SessionRequestRef::Close {
                session: session.clone(),
            },
            Self::ReconcileSession { identity, .. } => SessionRequestRef::QuerySession {
                identity: identity.clone(),
            },
            Self::ReconcileTurn {
                session, identity, ..
            } => SessionRequestRef::QueryTurn {
                session: session.clone(),
                identity: identity.clone(),
            },
            Self::ReplayStream {
                session,
                after_sequence,
                after_digest,
                limit,
                ..
            } => SessionRequestRef::Replay {
                session: session.clone(),
                after_sequence: *after_sequence,
                after_digest: after_digest.clone(),
                limit: *limit,
            },
            Self::AckStream {
                session,
                through_sequence,
                event_digest,
                ..
            } => SessionRequestRef::Ack {
                session: session.clone(),
                through_sequence: *through_sequence,
                event_digest: event_digest.clone(),
            },
        }
    }
}
