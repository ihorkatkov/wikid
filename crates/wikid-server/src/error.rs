//! HTTP error rendering (DESIGN §7): every failure — core errors, auth,
//! unknown wikis, malformed requests — becomes a stable status code with the
//! body `{"error":{"code","message","hint"}}`.

use axum::Json;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use wikid_core::WikidError;

/// A rendered API failure: an HTTP status plus the structured error body.
#[derive(Debug)]
pub struct ApiError {
	status: StatusCode,
	code: &'static str,
	message: String,
	hint: Option<String>,
}

#[derive(Serialize)]
struct ErrorBody<'a> {
	error: ErrorDetail<'a>,
}

#[derive(Serialize)]
struct ErrorDetail<'a> {
	code: &'a str,
	message: &'a str,
	#[serde(skip_serializing_if = "Option::is_none")]
	hint: Option<&'a str>,
}

impl ApiError {
	pub(crate) fn unauthorized() -> Self {
		Self {
			status: StatusCode::UNAUTHORIZED,
			code: "unauthorized",
			message: "missing or unknown bearer token".to_owned(),
			hint: Some("send 'Authorization: Bearer <token>' with a token from the daemon config".to_owned()),
		}
	}

	pub(crate) fn unknown_wiki<'a>(name: &str, available: impl Iterator<Item = &'a String>) -> Self {
		let names: Vec<&str> = available.map(String::as_str).collect();
		let listing = if names.is_empty() {
			"none".to_owned()
		} else {
			names.join(", ")
		};
		Self {
			status: StatusCode::NOT_FOUND,
			code: "unknown_wiki",
			message: format!("unknown wiki {name:?}; available: {listing}"),
			hint: Some("GET /v1/wikis lists the wikis this daemon serves".to_owned()),
		}
	}

	/// A malformed request (bad query/body, missing confirmation flag):
	/// the `usage` 400 category from DESIGN §7.
	pub(crate) fn usage(message: impl Into<String>, hint: Option<String>) -> Self {
		Self {
			status: StatusCode::BAD_REQUEST,
			code: "usage",
			message: message.into(),
			hint,
		}
	}

	pub(crate) fn route_not_found() -> Self {
		Self {
			status: StatusCode::NOT_FOUND,
			code: "not_found",
			message: "no such route".to_owned(),
			hint: Some("the API lives under /v1/wikis — GET /v1/wikis lists the wikis".to_owned()),
		}
	}

	pub(crate) fn method_not_allowed() -> Self {
		Self {
			status: StatusCode::METHOD_NOT_ALLOWED,
			code: "method_not_allowed",
			message: "method not allowed for this route".to_owned(),
			hint: None,
		}
	}
}

impl From<WikidError> for ApiError {
	fn from(err: WikidError) -> Self {
		// DESIGN §7 status mapping. `code`, `message`, and `hint` come from
		// the core error verbatim — surfaces never invent their own wording.
		let status = match &err {
			WikidError::NotFound { .. } | WikidError::NoMatch { .. } => StatusCode::NOT_FOUND,
			WikidError::InvalidPath { .. } | WikidError::BadPattern { .. } => StatusCode::BAD_REQUEST,
			WikidError::AlreadyExists { .. } | WikidError::Ambiguous { .. } => StatusCode::CONFLICT,
			WikidError::NotUtf8 { .. } => StatusCode::UNSUPPORTED_MEDIA_TYPE,
			WikidError::Io(_) => StatusCode::INTERNAL_SERVER_ERROR,
		};
		Self {
			status,
			code: err.code(),
			message: err.to_string(),
			hint: err.hint(),
		}
	}
}

impl From<QueryRejection> for ApiError {
	fn from(rejection: QueryRejection) -> Self {
		Self::usage(rejection.body_text(), None)
	}
}

impl From<JsonRejection> for ApiError {
	fn from(rejection: JsonRejection) -> Self {
		Self::usage(rejection.body_text(), None)
	}
}

impl IntoResponse for ApiError {
	fn into_response(self) -> Response {
		let body = Json(ErrorBody {
			error: ErrorDetail {
				code: self.code,
				message: &self.message,
				hint: self.hint.as_deref(),
			},
		});
		(self.status, body).into_response()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn wikid_errors_map_to_design_statuses() {
		let cases: Vec<(WikidError, StatusCode, &str)> = vec![
			(
				WikidError::NotFound { path: "a.md".into() },
				StatusCode::NOT_FOUND,
				"not_found",
			),
			(
				WikidError::NoMatch {
					path: "a.md".into(),
					nearest_line: None,
				},
				StatusCode::NOT_FOUND,
				"no_match",
			),
			(
				WikidError::InvalidPath {
					path: "../a".into(),
					reason: "escape".into(),
				},
				StatusCode::BAD_REQUEST,
				"invalid_path",
			),
			(
				WikidError::BadPattern {
					pattern: "(".into(),
					reason: "unclosed".into(),
				},
				StatusCode::BAD_REQUEST,
				"bad_pattern",
			),
			(
				WikidError::AlreadyExists { path: "a.md".into() },
				StatusCode::CONFLICT,
				"already_exists",
			),
			(
				WikidError::Ambiguous {
					path: "a.md".into(),
					count: 2,
				},
				StatusCode::CONFLICT,
				"ambiguous",
			),
			(
				WikidError::NotUtf8 { path: "a.png".into() },
				StatusCode::UNSUPPORTED_MEDIA_TYPE,
				"not_utf8",
			),
			(
				WikidError::Io(std::io::Error::other("disk")),
				StatusCode::INTERNAL_SERVER_ERROR,
				"io",
			),
		];
		for (err, status, code) in cases {
			let api = ApiError::from(err);
			assert_eq!(api.status, status, "wrong status for {code}");
			assert_eq!(api.code, code);
		}
	}

	#[test]
	fn body_omits_hint_when_absent() {
		let with = serde_json::to_string(&ErrorBody {
			error: ErrorDetail {
				code: "io",
				message: "disk",
				hint: Some("retry"),
			},
		})
		.unwrap();
		assert_eq!(with, r#"{"error":{"code":"io","message":"disk","hint":"retry"}}"#);
		let without = serde_json::to_string(&ErrorBody {
			error: ErrorDetail {
				code: "io",
				message: "disk",
				hint: None,
			},
		})
		.unwrap();
		assert_eq!(without, r#"{"error":{"code":"io","message":"disk"}}"#);
	}

	#[test]
	fn unknown_wiki_lists_available_names() {
		let names = ["notes".to_owned(), "projects".to_owned()];
		let err = ApiError::unknown_wiki("nope", names.iter());
		assert_eq!(err.status, StatusCode::NOT_FOUND);
		assert_eq!(err.code, "unknown_wiki");
		assert!(err.message.contains("notes, projects"), "got: {}", err.message);
		let err = ApiError::unknown_wiki("nope", [].iter());
		assert!(err.message.contains("available: none"));
	}
}
