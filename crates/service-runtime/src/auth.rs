//! Authenticated execution context and realm policy.

use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::eventlog::ServiceId;

macro_rules! identity_type {
    ($name:ident, $label:literal) => {
        #[doc = concat!("A validated ", $label, " identifier.")]
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            #[doc = concat!("Creates a ", $label, " identifier.")]
            pub fn new(value: impl Into<String>) -> Result<Self, InvalidIdentity> {
                let value = value.into();
                if value.trim().is_empty() {
                    return Err(InvalidIdentity { kind: $label });
                }
                Ok(Self(value))
            }

            #[doc = concat!("Returns the ", $label, " identifier as text.")]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(serde::de::Error::custom)
            }
        }
    };
}

/// An invalid identity field.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("{kind} identifier must not be empty")]
pub struct InvalidIdentity {
    kind: &'static str,
}

identity_type!(TenantId, "tenant");
identity_type!(AuthorityId, "authority");
identity_type!(UserId, "user");
identity_type!(ExecutorId, "executor");
identity_type!(RealmId, "realm");

/// Identity claims produced by a trusted authentication adapter.
///
/// This type is intentionally not deserializable. A transport adapter must verify its untrusted
/// input before constructing these parts and promoting them to [`VerifiedAuthContext`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct VerifiedIdentity {
    tenant: TenantId,
    authority: AuthorityId,
    user: UserId,
    executor: Option<ExecutorId>,
    realm: Option<RealmId>,
}

impl VerifiedIdentity {
    /// Records identity parts after a transport authentication adapter has verified them.
    pub fn after_verification(
        tenant: TenantId,
        authority: AuthorityId,
        user: UserId,
        executor: Option<ExecutorId>,
        realm: Option<RealmId>,
    ) -> Self {
        Self {
            tenant,
            authority,
            user,
            executor,
            realm,
        }
    }
}

/// Authenticated identity available to application execution.
///
/// There is no route or request representation here: realm is an optional, verified identity
/// claim. In particular, an absent realm remains different from the literal realm `default`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct VerifiedAuthContext(VerifiedIdentity);

impl VerifiedAuthContext {
    /// Promotes claims that an authentication adapter has already verified.
    pub fn from_verified(identity: VerifiedIdentity) -> Self {
        Self(identity)
    }

    /// Returns the verified tenant.
    pub fn tenant(&self) -> &TenantId {
        &self.0.tenant
    }

    /// Returns the verified authority.
    pub fn authority(&self) -> &AuthorityId {
        &self.0.authority
    }

    /// Returns the verified user.
    pub fn user(&self) -> &UserId {
        &self.0.user
    }

    /// Returns the verified executor, when another human or agent applied the intent.
    pub fn executor(&self) -> Option<&ExecutorId> {
        self.0.executor.as_ref()
    }

    /// Returns the realm supplied by authentication, if one was supplied.
    pub fn realm(&self) -> Option<&RealmId> {
        self.0.realm.as_ref()
    }
}

/// The realm contract declared by a service.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RealmPolicy {
    /// Authentication must supply a realm.
    Required,
    /// Authentication may supply a realm.
    Optional,
    /// Authentication must not supply a realm.
    Forbidden,
}

impl RealmPolicy {
    /// Enforces this policy against an already verified context.
    pub fn enforce(self, context: &VerifiedAuthContext) -> Result<(), ContextPolicyViolation> {
        match (self, context.realm()) {
            (Self::Required, None) => Err(ContextPolicyViolation::RealmRequired),
            (Self::Forbidden, Some(realm)) => Err(ContextPolicyViolation::RealmForbidden {
                realm: realm.clone(),
            }),
            _ => Ok(()),
        }
    }
}

/// A verified context does not satisfy the service context policy.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ContextPolicyViolation {
    /// The service requires a realm but authentication did not supply one.
    #[error("the service requires an authenticated realm")]
    RealmRequired,
    /// The service forbids realms but authentication supplied one.
    #[error("the service forbids authenticated realm {realm}")]
    RealmForbidden {
        /// The supplied realm.
        realm: RealmId,
    },
    /// A stream belongs to another standalone service.
    #[error("stream service {stream_service} differs from runtime service {runtime_service}")]
    ServiceMismatch {
        /// Service encoded in the stream identity.
        stream_service: ServiceId,
        /// Stable service identity declared by the runtime implementation.
        runtime_service: ServiceId,
    },
    /// A stream belongs to a different tenant than the verified context.
    #[error("stream tenant {stream_tenant} differs from authenticated tenant {context_tenant}")]
    TenantMismatch {
        /// Tenant encoded in the stream identity.
        stream_tenant: TenantId,
        /// Tenant supplied by authentication.
        context_tenant: TenantId,
    },
    /// A stream belongs to a different exact optional realm than the verified context.
    #[error("stream realm {stream_realm:?} differs from authenticated realm {context_realm:?}")]
    RealmMismatch {
        /// Exact optional realm encoded in the stream identity.
        stream_realm: Option<RealmId>,
        /// Exact optional realm supplied by authentication.
        context_realm: Option<RealmId>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(realm: Option<&str>) -> VerifiedAuthContext {
        VerifiedAuthContext::from_verified(VerifiedIdentity::after_verification(
            TenantId::new("tenant-a").unwrap(),
            AuthorityId::new("authority-a").unwrap(),
            UserId::new("user-a").unwrap(),
            Some(ExecutorId::new("agent-a").unwrap()),
            realm.map(|value| RealmId::new(value).unwrap()),
        ))
    }

    #[test]
    fn required_rejects_absence_and_accepts_default() {
        assert_eq!(
            RealmPolicy::Required.enforce(&context(None)),
            Err(ContextPolicyViolation::RealmRequired)
        );
        assert!(
            RealmPolicy::Required
                .enforce(&context(Some("default")))
                .is_ok()
        );
    }

    #[test]
    fn optional_preserves_absent_and_default_as_distinct() {
        let absent = context(None);
        let default = context(Some("default"));

        assert!(RealmPolicy::Optional.enforce(&absent).is_ok());
        assert!(RealmPolicy::Optional.enforce(&default).is_ok());
        assert_eq!(absent.realm(), None);
        assert_eq!(default.realm().map(RealmId::as_str), Some("default"));
        assert_ne!(absent, default);
    }

    #[test]
    fn forbidden_rejects_any_present_realm() {
        assert!(RealmPolicy::Forbidden.enforce(&context(None)).is_ok());
        assert_eq!(
            RealmPolicy::Forbidden.enforce(&context(Some("default"))),
            Err(ContextPolicyViolation::RealmForbidden {
                realm: RealmId::new("default").unwrap()
            })
        );
    }

    #[test]
    fn executor_is_optional_without_synthesizing_the_user_or_authority() {
        let identity = VerifiedIdentity::after_verification(
            TenantId::new("tenant-a").unwrap(),
            AuthorityId::new("authority-a").unwrap(),
            UserId::new("user-a").unwrap(),
            None,
            None,
        );
        let context = VerifiedAuthContext::from_verified(identity);
        assert_eq!(context.executor(), None);
        assert_eq!(context.user().as_str(), "user-a");
        assert_eq!(context.authority().as_str(), "authority-a");
    }

    #[test]
    fn deserialized_identity_components_still_validate() {
        assert!(serde_json::from_str::<TenantId>(r#""tenant-a""#).is_ok());
        assert!(serde_json::from_str::<TenantId>(r#""  ""#).is_err());
    }
}
