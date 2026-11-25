# Account feature availability

`api.users().availability().await?` reads the signed-in account and returns `AccountAvailability`. `account.availability()` evaluates an existing account result without another request. The account country is trimmed and normalized to uppercase. It is not inferred from IP location, application language or the playback device.

Each feature returns `Availability::Available`, `Unavailable(FeatureRestriction)` or `Unknown`. `require("feature_name")` converts unavailable or unknown results into a structured `Error::FeatureUnavailable`. Transport, authentication and rate-limit errors propagate separately. A 502 never becomes a regional denial.

## Audiobook policy

SpotCore currently uses the following established-market allowlist:

```text
AE AT AU BE CA CH DE DK FI FR GB IE IS LI LU MC NL NZ SA SE US ZA
```

This is a conservative SDK policy, not a claim that Spotify has not activated other countries. A positive account flag cannot override it. An account outside this list receives `UnverifiedMarket`; an explicit negative account flag receives `AccountDenied`. A missing country or capability remains unknown. A listed country plus a positive audiobook capability enables the catalog feature.

`podcasts().audiobook_info()` applies this policy before returning typed metadata. Low-level show/audiobook metadata methods remain available for compatibility and diagnosis; their success does not prove entitlement. `play_audiobook()` also checks policy and chapter access. The server remains authoritative for the requested title and playback operation.

Catalog availability, purchased title access, included listening time and remaining allowance are different facts. `included_audiobook_time` stays unknown because an account's Premium tier alone does not identify Student plans, Family/Duo manager status or remaining allowance. Do not infer those facts from recommendations, search results or `hasAudiobooksAvailable`.

## Podcasts and Premium

Podcasts follow the server's account capability. No secondary-source podcast exclusion list is enforced. A missing flag stays unknown.

Native playback and hosting a Jam require Premium. These results describe account eligibility, not installed engine readiness or current device permissions. Runtime operations still report their own errors. Guest membership and permission to listen on the host are separate from permission to host a Jam.

## Sources and policy maintenance

Reviewed on 2026-11-25:

- [Spotify's established redemption markets](https://support.spotify.com/us/authors/article/redemption-codes/) names the 22 countries above and explicitly makes title access subject to territorial rights. It is evidence for this conservative baseline, not a universal entitlement guarantee.
- [Audiobooks going global](https://newsroom.spotify.com/2026-10-07/audiobooks-global-expansion/) announces expansion from 22 to more than 180 markets through the end of 2026. It does not establish which countries are already active.
- [Audiobooks in Premium plans](https://support.spotify.com/us/article/audiobooks-premium-plans/) distinguishes eligible plans, managers and listening allowance.

To expand the allowlist, confirm activation from official evidence, update `AUDIOBOOK_ESTABLISHED_MARKETS` and the regression cases, and document the evidence in the same commit. The list ships with the SDK; there is no remote policy service. Do not silently enable all announced markets on a date.
