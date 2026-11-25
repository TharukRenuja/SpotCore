//! Spotify display-segment markers, distinct from playable audiobook chapters.
use crate::Api;
use serde::{Deserialize, Serialize};
use spotcore_core::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChapterMarker {
    pub start_ms: u32,
    pub title: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ChapterSource {
    Spotify,
    Description,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChapterMarkers {
    pub markers: Vec<ChapterMarker>,
    pub source: ChapterSource,
}

fn invalid() -> Error {
    Error::Backend("invalid Spotify chapter metadata".into())
}
fn varint(bytes: &[u8], offset: &mut usize) -> Result<u64, Error> {
    let mut value = 0u64;
    for shift in (0..70).step_by(7) {
        let byte = *bytes.get(*offset).ok_or_else(invalid)?;
        *offset += 1;
        if shift == 63 && byte > 1 {
            return Err(invalid());
        }
        value |= u64::from(byte & 127) << shift;
        if byte < 128 {
            return Ok(value);
        }
    }
    Err(invalid())
}
struct Field<'a> {
    tag: u64,
    number: Option<u64>,
    bytes: &'a [u8],
}
fn fields(bytes: &[u8]) -> Result<Vec<Field<'_>>, Error> {
    if bytes.len() > 16 * 1024 * 1024 {
        return Err(invalid());
    }
    let mut offset = 0;
    let mut result = Vec::new();
    while offset < bytes.len() {
        if result.len() >= 4096 {
            return Err(invalid());
        }
        let key = varint(bytes, &mut offset)?;
        let tag = key >> 3;
        if tag == 0 {
            return Err(invalid());
        }
        let (number, length) = match key & 7 {
            0 => (Some(varint(bytes, &mut offset)?), 0),
            1 => (None, 8),
            2 => (
                None,
                usize::try_from(varint(bytes, &mut offset)?).map_err(|_| invalid())?,
            ),
            5 => (None, 4),
            _ => return Err(invalid()),
        };
        let end = offset
            .checked_add(length)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(invalid)?;
        result.push(Field {
            tag,
            number,
            bytes: &bytes[offset..end],
        });
        offset = end;
    }
    Ok(result)
}
fn number(fields: &[Field<'_>], tag: u64) -> Result<u32, Error> {
    fields
        .iter()
        .find(|field| field.tag == tag)
        .map_or(Ok(0), |field| {
            u32::try_from(field.number.ok_or_else(invalid)?).map_err(|_| invalid())
        })
}
fn text<'a>(fields: &[Field<'a>], tag: u64) -> Result<&'a str, Error> {
    std::str::from_utf8(
        fields
            .iter()
            .find(|field| field.tag == tag)
            .map_or(&[], |field| field.bytes),
    )
    .map_err(|_| invalid())
}
fn encode_varint(mut value: u64, out: &mut Vec<u8>) {
    while value >= 128 {
        out.push((value as u8 & 127) | 128);
        value >>= 7;
    }
    out.push(value as u8);
}
fn message(tag: u64, bytes: &[u8]) -> Vec<u8> {
    let mut result = Vec::new();
    encode_varint(tag * 8 + 2, &mut result);
    encode_varint(bytes.len() as u64, &mut result);
    result.extend_from_slice(bytes);
    result
}
fn request(uri: &str) -> Result<Vec<u8>, Error> {
    let id = uri
        .strip_prefix("spotify:episode:")
        .or_else(|| uri.strip_prefix("spotify:chapter:"));
    if !id.is_some_and(|id| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_alphanumeric())) {
        return Err(Error::InvalidInput(
            "chapter markers require an episode or chapter URI",
        ));
    }
    let mut entity = message(1, uri.as_bytes());
    entity.extend(message(2, &[8, 38]));
    Ok(message(2, &entity))
}
fn decode(bytes: &[u8], expected: &str) -> Result<Vec<ChapterMarker>, Error> {
    for array in fields(bytes)?.iter().filter(|field| field.tag == 2) {
        let array = fields(array.bytes)?;
        if number(&array, 2)? != 38 {
            continue;
        }
        for entity in array.iter().filter(|field| field.tag == 3) {
            let entity = fields(entity.bytes)?;
            if text(&entity, 2)? != expected {
                continue;
            }
            if let Some(header) = entity.iter().find(|field| field.tag == 1) {
                match number(&fields(header.bytes)?, 1)? {
                    0 | 200 => {}
                    204 | 404 => return Ok(vec![]),
                    status => {
                        return Err(Error::Http {
                            status: status.try_into().map_err(|_| invalid())?,
                            retry_after: None,
                        });
                    }
                }
            }
            let Some(any) = entity.iter().find(|field| field.tag == 3) else {
                return Ok(vec![]);
            };
            let any = fields(any.bytes)?;
            if !text(&any, 1)?.ends_with("spotify.displaysegments.v1.DisplaySegmentsExtension") {
                return Err(invalid());
            }
            let payload = fields(
                any.iter()
                    .find(|field| field.tag == 2)
                    .ok_or_else(invalid)?
                    .bytes,
            )?;
            if text(&payload, 1)? != expected {
                return Err(invalid());
            }
            let duration = number(&payload, 4)?;
            let mut result: Vec<ChapterMarker> = Vec::new();
            for segment in payload.iter().filter(|field| field.tag == 2) {
                if result.len() >= 1000 {
                    return Err(invalid());
                }
                let segment = fields(segment.bytes)?;
                let start_ms = number(&segment, 4)?;
                if duration > 0 && start_ms >= duration
                    || result.last().is_some_and(|last| last.start_ms >= start_ms)
                {
                    return Err(invalid());
                }
                result.push(ChapterMarker {
                    start_ms,
                    title: text(&segment, 6)?.into(),
                });
            }
            return Ok(result);
        }
    }
    Ok(vec![])
}
fn timestamp(text: &str) -> Option<u32> {
    let parts: Vec<_> = text.split(':').collect();
    if !(2..=3).contains(&parts.len()) {
        return None;
    }
    let mut seconds = 0u32;
    for (index, part) in parts.iter().enumerate() {
        if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let value: u32 = part.parse().ok()?;
        if index > 0 && value >= 60 {
            return None;
        }
        seconds = seconds.checked_mul(60)?.checked_add(value)?;
    }
    seconds.checked_mul(1000)
}
pub(crate) fn from_description(description: &str, duration: u32) -> Vec<ChapterMarker> {
    if description.len() > 1024 * 1024 {
        return vec![];
    }
    let mut plain = String::new();
    let mut rest = description;
    while let Some(start) = rest.find('<') {
        plain.push_str(&rest[..start]);
        let Some(end) = rest[start..].find('>') else {
            return vec![];
        };
        let tag = rest[start + 1..start + end].trim().to_ascii_lowercase();
        if matches!(
            tag.split_whitespace()
                .next()
                .unwrap_or_default()
                .trim_end_matches('/'),
            "br" | "/p" | "/div" | "/li"
        ) {
            plain.push('\n');
        }
        rest = &rest[start + end + 1..];
    }
    plain.push_str(rest);
    let plain = plain.replace("&nbsp;", " ").replace("&amp;", "&");
    let mut result: Vec<ChapterMarker> = Vec::new();
    for line in plain.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let (stamp, title) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        if !stamp.contains(':')
            || !stamp
                .trim_start_matches('(')
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_digit)
        {
            continue;
        }
        if stamp.starts_with('(') != stamp.ends_with(')') {
            return vec![];
        }
        let Some(start_ms) = timestamp(stamp.trim_start_matches('(').trim_end_matches(')')) else {
            return vec![];
        };
        let title = title
            .trim()
            .trim_start_matches(['-', '\u{2013}', '\u{2014}'])
            .trim();
        if title.is_empty()
            || start_ms >= duration
            || result
                .last()
                .is_some_and(|last| start_ms < last.start_ms.saturating_add(30_000))
        {
            return vec![];
        }
        result.push(ChapterMarker {
            start_ms,
            title: title.into(),
        });
    }
    if result.len() < 3 || result[0].start_ms != 0 {
        return vec![];
    }
    result
}
pub(crate) async fn load(api: &Api, uri: &str) -> Result<ChapterMarkers, Error> {
    let body = request(uri)?;
    let response = api
        .request_raw(
            "POST",
            "https://spclient.wg.spotify.com/extended-metadata/v0/extended-metadata",
            Some(body),
            "application/x-protobuf",
        )
        .await;
    match response {
        Ok(response) => {
            let markers = decode(&response.body, uri)?;
            if !markers.is_empty() {
                return Ok(ChapterMarkers {
                    markers,
                    source: ChapterSource::Spotify,
                });
            }
        }
        Err(Error::Http {
            status: 404 | 204, ..
        }) => {}
        Err(error) => return Err(error),
    }
    let item = api.podcasts().episode(uri).await?;
    let description = item
        .raw
        .get("description")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    Ok(ChapterMarkers {
        markers: from_description(
            description,
            item.duration_ms.unwrap_or(0).min(u64::from(u32::MAX)) as u32,
        ),
        source: ChapterSource::Description,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extended_metadata_decodes_only_the_requested_item() {
        let uri = "spotify:episode:test";
        let mut segment = vec![32, 0];
        segment.extend(message(6, b"Intro"));
        let mut payload = message(1, uri.as_bytes());
        payload.extend(message(2, &segment));
        payload.extend_from_slice(&[32, 120]);
        let mut any = message(
            1,
            b"type.googleapis.com/spotify.displaysegments.v1.DisplaySegmentsExtension",
        );
        any.extend(message(2, &payload));
        let mut entity = message(2, uri.as_bytes());
        entity.extend(message(3, &any));
        let mut array = vec![16, 38];
        array.extend(message(3, &entity));
        let response = message(2, &array);
        assert_eq!(
            decode(&response, uri).unwrap(),
            vec![ChapterMarker {
                start_ms: 0,
                title: "Intro".into()
            }]
        );
        assert!(
            decode(&response, "spotify:episode:another")
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn malformed_wire_is_rejected_and_spoken_uri_is_validated() {
        assert!(fields(&[0x80]).is_err());
        assert!(fields(&[0]).is_err());
        assert!(fields(&[10, 255, 255, 255, 255, 127]).is_err());
        assert!(request("spotify:track:a").is_err());
        assert!(request("spotify:episode:a").is_ok());
    }
    #[test]
    fn manual_markers_require_order_zero_start_and_three_entries() {
        let valid = "<p>00:00 Intro</p><p>01:00 Topic</p><p>02:00 End</p>";
        assert_eq!(from_description(valid, 180_000).len(), 3);
        assert!(from_description("00:01 Intro\n01:00 Topic\n02:00 End", 180_000).is_empty());
        assert!(from_description("00:00 Intro\n00:20 Topic\n02:00 End", 180_000).is_empty());
    }
}
