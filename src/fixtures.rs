//! Bounded response fixtures for recovery-run steps.
use std::{collections::BTreeMap, fmt};

use axum::{
    body::{Body, Bytes},
    http::{HeaderName, HeaderValue, StatusCode, header},
    response::Response,
};
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{self, MapAccess, Visitor},
};

use crate::error::{ApiError, with_simulation_headers};

pub(crate) const MAX_FIXTURE_BODY_BYTES: usize = 4096;
pub(crate) const MAX_FIXTURE_HEADERS: usize = 5;
pub(crate) const MAX_FIXTURE_HEADER_VALUE_BYTES: usize = 256;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Fixture {
    body: String,
    #[serde(default = "default_content_type")]
    content_type: String,
    #[serde(default, deserialize_with = "deserialize_headers")]
    headers: BTreeMap<String, String>,
}

fn default_content_type() -> String {
    "application/json".into()
}

// A normal map silently replaces repeated JSON keys. Reject them before that
// information is lost, including names that differ only in ASCII case.
fn deserialize_headers<'de, D>(deserializer: D) -> Result<BTreeMap<String, String>, D::Error>
where
    D: Deserializer<'de>,
{
    struct HeadersVisitor;

    impl<'de> Visitor<'de> for HeadersVisitor {
        type Value = BTreeMap<String, String>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("an object of unique HTTP header names and string values")
        }

        fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
        where
            M: MapAccess<'de>,
        {
            let mut headers = BTreeMap::new();
            while let Some((name, value)) = map.next_entry::<String, String>()? {
                let name = name.to_ascii_lowercase();
                if headers.insert(name, value).is_some() {
                    return Err(de::Error::custom("fixture headers must have unique names"));
                }
                if headers.len() > MAX_FIXTURE_HEADERS {
                    return Err(de::Error::custom("fixture permits at most 5 headers"));
                }
            }
            Ok(headers)
        }
    }

    deserializer.deserialize_map(HeadersVisitor)
}

fn invalid(message: &'static str) -> ApiError {
    ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, message)
}

impl Fixture {
    pub(crate) fn validate(&self) -> Result<(), ApiError> {
        if self.body.len() > MAX_FIXTURE_BODY_BYTES {
            return Err(invalid(
                "fixture body must contain at most 4096 UTF-8 bytes",
            ));
        }
        let media_type = self
            .content_type
            .strip_suffix("; charset=utf-8")
            .unwrap_or(&self.content_type);
        if !matches!(
            media_type,
            "application/json" | "text/plain" | "application/xml"
        ) {
            return Err(invalid(
                "fixture content_type must be application/json, text/plain, or application/xml, optionally followed by ; charset=utf-8",
            ));
        }
        if self.headers.len() > MAX_FIXTURE_HEADERS {
            return Err(invalid("fixture permits at most 5 headers"));
        }
        for (name, value) in &self.headers {
            if !matches!(
                name.as_str(),
                "retry-after" | "www-authenticate" | "allow" | "cache-control" | "x-request-id"
            ) {
                return Err(invalid(
                    "fixture header must be retry-after, www-authenticate, allow, cache-control, or x-request-id",
                ));
            }
            if value.len() > MAX_FIXTURE_HEADER_VALUE_BYTES
                || !value.bytes().all(|byte| (b' '..=b'~').contains(&byte))
            {
                return Err(invalid(
                    "fixture header values must contain at most 256 printable ASCII bytes",
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn bytes(&self) -> Bytes {
        Bytes::copy_from_slice(self.body.as_bytes())
    }

    /// Build a response from a fixture validated when the run was created.
    pub(crate) fn response(&self, status: StatusCode) -> Response {
        let mut response = Response::new(Body::from(self.bytes()));
        *response.status_mut() = status;
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_str(&self.content_type).expect("validated fixture content type"),
        );
        let mut response = with_simulation_headers(response);
        for (name, value) in &self.headers {
            response.headers_mut().insert(
                HeaderName::from_bytes(name.as_bytes()).expect("validated fixture header name"),
                HeaderValue::from_str(value).expect("validated fixture header value"),
            );
        }
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture(value: serde_json::Value) -> Fixture {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn fixture_defaults_preserve_exact_decoded_bytes() {
        let fixture = fixture(json!({"body": " { \"text\": \"café 💤\" }\n"}));
        assert!(fixture.validate().is_ok());
        assert_eq!(fixture.bytes(), " { \"text\": \"café 💤\" }\n".as_bytes());
        assert_eq!(fixture.content_type, "application/json");
        assert!(fixture.headers.is_empty());
    }

    #[test]
    fn fixture_body_limit_counts_utf8_bytes_and_allows_empty_bodies() {
        for body in [String::new(), "x".repeat(4096), "é".repeat(2048)] {
            assert!(fixture(json!({"body":body})).validate().is_ok());
        }
        for body in ["x".repeat(4097), "é".repeat(2049)] {
            assert!(fixture(json!({"body":body})).validate().is_err());
        }
    }

    #[test]
    fn fixture_content_types_are_explicit_and_cannot_inject_headers() {
        for media_type in ["application/json", "text/plain", "application/xml"] {
            for suffix in ["", "; charset=utf-8"] {
                assert!(
                    fixture(json!({"body":"", "content_type":format!("{media_type}{suffix}")}))
                        .validate()
                        .is_ok()
                );
            }
        }
        for media_type in [
            "text/html",
            "application/json; charset=ascii",
            "application/json; charset=utf-8; charset=utf-8",
            "text/plain\r\nX-Foo: bar",
            "text/plain\0",
        ] {
            assert!(
                fixture(json!({"body":"", "content_type":media_type}))
                    .validate()
                    .is_err()
            );
        }
    }

    #[test]
    fn fixture_headers_normalize_names_and_reject_duplicates() {
        let fixture = fixture(json!({"body":"", "headers":{"X-Request-ID":"request-1"}}));
        assert!(fixture.validate().is_ok());
        assert_eq!(fixture.headers["x-request-id"], "request-1");
        for json in [
            r#"{"body":"","headers":{"allow":"GET","allow":"POST"}}"#,
            r#"{"body":"","headers":{"Allow":"GET","ALLOW":"POST"}}"#,
        ] {
            assert!(serde_json::from_str::<Fixture>(json).is_err());
        }
    }

    #[test]
    fn fixture_headers_reject_controls_non_ascii_framing_and_cors() {
        for name in [
            "content-length",
            "transfer-encoding",
            "connection",
            "content-type",
            "access-control-allow-origin",
            "set-cookie",
            "location",
            "x-custom",
            "rétry-after",
            "allow\n",
        ] {
            assert!(
                fixture(json!({"body":"", "headers":{name:"1"}}))
                    .validate()
                    .is_err()
            );
        }
        for value in ["a\rb", "a\nb", "a\tb", "a\0b", "é", "\u{7f}"] {
            assert!(
                fixture(json!({"body":"", "headers":{"x-request-id":value}}))
                    .validate()
                    .is_err()
            );
        }
        assert!(
            fixture(json!({"body":"", "headers":{"x-request-id":"x".repeat(256)}}))
                .validate()
                .is_ok()
        );
        assert!(
            fixture(json!({"body":"", "headers":{"x-request-id":"x".repeat(257)}}))
                .validate()
                .is_err()
        );
    }
}
