use thiserror::Error;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum AuthError {
    #[error("尚未登录。请运行 `qidiwork login`。")]
    NotLoggedIn,

    /// Token expired and no refresh authority available.
    #[error("登录令牌已过期。请运行 `qidiwork login` 重新登录。")]
    TokenExpiredNoRefresh,

    /// Server rejected the token (401) with no recovery path.
    #[error("服务端拒绝了认证。请运行 `qidiwork login` 重新登录。")]
    ServerRejectedNoRecovery,

    /// All recovery strategies exhausted.
    #[error("认证恢复已尝试全部策略，需要重新登录。")]
    RecoveryExhausted,

    /// A session's team principal violates the `force_login_team_uuid` pin.
    /// `message` states which team is required vs. returned.
    #[error("{message} 请运行 `qidiwork login` 登录到指定的团队。")]
    PinnedTeamMismatch { message: String },

    /// Cached API-key session rejected because API-key auth is disabled.
    #[error("管理员已禁用 API Key 认证。请运行 `qidiwork login` 登录。")]
    ApiKeyAuthDisabled,

    /// Outcome of a refresh-authority attempt. Recoverability (and, for
    /// permanent failures, the reason) lives in [`RefreshTokenError`].
    #[error(transparent)]
    Refresh(#[from] RefreshTokenError),
}

/// Recoverability axis of a token-refresh attempt. Deliberately total (no
/// `#[non_exhaustive]`): "permanent vs transient" is a closed decision every
/// caller must make, so a future third state should break consumers loudly.
#[derive(Debug, Error)]
pub enum RefreshTokenError {
    /// The credential is dead; the user must re-authenticate.
    #[error(transparent)]
    Permanent(#[from] RefreshTokenFailedError),
    /// Network / 5xx / unknown blip; safe to retry later. Carries the cause.
    #[error(transparent)]
    Transient(RefreshTransientError),
}

/// A retryable refresh failure, wrapping its cause. No public `From`:
/// construct only via [`AuthError::transient`] /
/// [`AuthError::transient_source`], so a stray `?` on some error can't silently
/// classify a permanent failure as retryable (mirrors the dedicated
/// [`RefreshTokenFailedError`] on the permanent arm). Display frames the cause
/// as an auth-refresh failure so internal messages (lock timeout, sleep defer)
/// don't surface bare; the permanent arm derives its copy from
/// [`RefreshTokenFailedReason::user_message`] and is not prefixed.
#[derive(Debug, Error)]
#[error("auth refresh failed: {0}")]
pub struct RefreshTransientError(#[source] Box<dyn std::error::Error + Send + Sync>);

/// A terminal refresh failure. `reason` is machine-readable; the user-facing
/// copy is derived from it via [`RefreshTokenFailedReason::user_message`], so
/// the two can never drift.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{}", .reason.user_message())]
#[non_exhaustive]
pub struct RefreshTokenFailedError {
    pub reason: RefreshTokenFailedReason,
}

impl From<RefreshTokenFailedReason> for RefreshTokenFailedError {
    fn from(reason: RefreshTokenFailedReason) -> Self {
        Self { reason }
    }
}

/// Why a token refresh terminally failed, grounded in the OAuth2 error codes
/// our IdP actually emits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RefreshTokenFailedReason {
    /// `invalid_grant` — the refresh token is no longer valid (expired, reused,
    /// or revoked; the IdP does not distinguish these).
    RefreshTokenRejected,
    /// `invalid_client` — the client/app credential was rejected.
    ClientRejected,
    /// Escalation from repeated transient failures (OIDC) or a single
    /// external-binary failure. Never a raw IdP code: an unrecognized terminal
    /// code is classified transient, not `Other` (see `classify_terminal`).
    Other,
}

impl RefreshTokenFailedReason {
    /// Sticky until the credential changes (never ages out): a revoked refresh
    /// token never self-heals, whereas client rotation / transient escalation
    /// recover, so those age out past the TTL.
    pub(crate) fn is_sticky(self) -> bool {
        match self {
            Self::RefreshTokenRejected => true,
            Self::ClientRejected | Self::Other => false,
        }
    }

    /// User-facing copy for a terminal refresh failure; the raw IdP code stays
    /// in logs.
    pub(crate) fn user_message(self) -> &'static str {
        match self {
            Self::RefreshTokenRejected => {
                "你的会话已过期。请运行 `qidiwork login` 重新登录。"
            }
            Self::ClientRejected => {
                "认证暂时不可用。如果持续如此，请运行 `qidiwork login`。"
            }
            Self::Other => {
                "无法刷新认证。请运行 `qidiwork login` 重新登录。"
            }
        }
    }
}

impl AuthError {
    /// A retryable refresh failure with a message-only cause, for the genuinely
    /// message-only sites (lock timeout, sleep/dark-wake defer, no refresher);
    /// use [`Self::transient_source`] when a real error is in hand.
    pub(crate) fn transient(message: impl Into<String>) -> Self {
        Self::transient_source(message.into())
    }

    /// A retryable refresh failure that preserves `source` in the error chain
    /// (`Transient` carries the cause), so callers with a real error don't
    /// flatten it to a string.
    pub(crate) fn transient_source(
        source: impl Into<Box<dyn std::error::Error + Send + Sync>>,
    ) -> Self {
        AuthError::Refresh(RefreshTokenError::Transient(RefreshTransientError(
            source.into(),
        )))
    }

    /// A terminal refresh failure for an already-classified `reason`.
    pub(crate) fn permanent(reason: RefreshTokenFailedReason) -> Self {
        AuthError::Refresh(RefreshTokenError::Permanent(reason.into()))
    }
}
