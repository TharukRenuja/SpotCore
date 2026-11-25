//! Account policy is separate from title entitlement and engine capability.
use crate::Account;
use serde::{Deserialize, Serialize};
use spotcore_auth::AccountTier;
use spotcore_core::{Error, FeatureRestriction};

/// Conservative established-market policy. See docs/availability.md for sources and scope.
pub const AUDIOBOOK_ESTABLISHED_MARKETS: &[&str] = &[
    "AE", "AT", "AU", "BE", "CA", "CH", "DE", "DK", "FI", "FR", "GB", "IE", "IS", "LI", "LU", "MC",
    "NL", "NZ", "SA", "SE", "US", "ZA",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Availability {
    Available,
    Unavailable(FeatureRestriction),
    Unknown,
}
impl Availability {
    pub fn require(&self, feature: &str) -> Result<(), Error> {
        match self {
            Self::Available => Ok(()),
            Self::Unavailable(reason) => Err(Error::FeatureUnavailable {
                feature: feature.into(),
                reason: reason.clone(),
            }),
            Self::Unknown => Err(Error::FeatureUnavailable {
                feature: feature.into(),
                reason: FeatureRestriction::AvailabilityUnknown,
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountAvailability {
    pub country: Option<String>,
    pub podcasts: Availability,
    pub audiobooks: Availability,
    pub native_playback: Availability,
    pub jam_hosting: Availability,
    /// Premium alone does not identify Student, plan-manager status or remaining hours.
    pub included_audiobook_time: Availability,
}

impl Account {
    pub fn availability(&self) -> AccountAvailability {
        let country = self
            .country
            .as_deref()
            .map(str::trim)
            .filter(|country| {
                country.len() == 2 && country.bytes().all(|byte| byte.is_ascii_alphabetic())
            })
            .map(str::to_ascii_uppercase);
        let flag = |name: &str| {
            self.raw
                .get("account")?
                .get("attributes")?
                .get(name)?
                .as_bool()
        };
        let podcasts = match flag("hasPodcastsAvailable") {
            Some(true) => Availability::Available,
            Some(false) => Availability::Unavailable(FeatureRestriction::AccountDenied),
            None => Availability::Unknown,
        };
        let audiobooks = if flag("hasAudiobooksAvailable") == Some(false) {
            Availability::Unavailable(FeatureRestriction::AccountDenied)
        } else {
            match country.as_deref() {
                Some(code) if !AUDIOBOOK_ESTABLISHED_MARKETS.contains(&code) => {
                    Availability::Unavailable(FeatureRestriction::UnverifiedMarket {
                        country: code.into(),
                    })
                }
                Some(_) if flag("hasAudiobooksAvailable") == Some(true) => Availability::Available,
                _ => Availability::Unknown,
            }
        };
        let premium = match self.tier {
            AccountTier::Premium => Availability::Available,
            AccountTier::Free => Availability::Unavailable(FeatureRestriction::RequiresPremium),
            _ => Availability::Unknown,
        };
        AccountAvailability {
            country,
            podcasts,
            audiobooks,
            native_playback: premium.clone(),
            jam_hosting: premium,
            included_audiobook_time: Availability::Unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn account(country: Option<&str>, enabled: Option<bool>) -> Account {
        Account {
            uri: "spotify:user:test".into(),
            username: "test".into(),
            name: "Test".into(),
            tier: AccountTier::Premium,
            country: country.map(str::to_owned),
            images: vec![],
            raw: serde_json::json!({"account":{"attributes":{"hasAudiobooksAvailable":enabled}}}),
        }
    }
    #[test]
    fn established_policy_cannot_be_overridden_by_rollout_flag() {
        assert_eq!(
            account(Some("us"), Some(true)).availability().audiobooks,
            Availability::Available
        );
        assert!(matches!(
            account(Some("LK"), Some(true)).availability().audiobooks,
            Availability::Unavailable(FeatureRestriction::UnverifiedMarket { .. })
        ));
        assert_eq!(
            account(Some("US"), Some(false)).availability().audiobooks,
            Availability::Unavailable(FeatureRestriction::AccountDenied)
        );
        assert_eq!(
            account(None, Some(true)).availability().audiobooks,
            Availability::Unknown
        );
        assert_eq!(
            account(Some("US"), None).availability().audiobooks,
            Availability::Unknown
        );
        assert_eq!(
            account(Some("US"), Some(true))
                .availability()
                .included_audiobook_time,
            Availability::Unknown
        );
    }
    #[test]
    fn free_tier_does_not_remove_catalog_access() {
        let mut free = account(Some("US"), Some(true));
        free.tier = AccountTier::Free;
        assert_eq!(free.availability().audiobooks, Availability::Available);
        assert_eq!(
            free.availability().native_playback,
            Availability::Unavailable(FeatureRestriction::RequiresPremium)
        );
        assert!(matches!(
            free.availability()
                .native_playback
                .require("native_playback"),
            Err(Error::FeatureUnavailable {
                reason: FeatureRestriction::RequiresPremium,
                ..
            })
        ));
    }
}
