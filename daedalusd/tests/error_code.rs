//! P3.2 tests — ErrorCode mapping and serialization.

use daedalusd::error::{ErrorCode, ErrorKind, ProviderError};

#[test]
fn from_error_kind_cancelled() {
    assert_eq!(
        ErrorCode::from_error_kind(&ErrorKind::Cancelled),
        ErrorCode::Cancelled
    );
}

#[test]
fn from_error_kind_task_timeout() {
    assert_eq!(
        ErrorCode::from_error_kind(&ErrorKind::TaskTimeout),
        ErrorCode::TaskTimeout
    );
}

#[test]
fn from_error_kind_all_six() {
    let pairs = [
        (ErrorKind::Cancelled, ErrorCode::Cancelled),
        (ErrorKind::TaskTimeout, ErrorCode::TaskTimeout),
        (ErrorKind::ToolFailure, ErrorCode::ToolFailure),
        (ErrorKind::MaxIterations, ErrorCode::MaxIterations),
        (ErrorKind::ProviderExhausted, ErrorCode::ProviderExhausted),
        (ErrorKind::ProviderFatal, ErrorCode::ProviderFatal),
    ];
    for (kind, expected) in &pairs {
        assert_eq!(ErrorCode::from_error_kind(kind), *expected);
    }
}

#[test]
fn from_provider_error_auth() {
    let e = ProviderError::Auth {
        status: 401,
        body: "bad key".into(),
    };
    assert_eq!(ErrorCode::from_provider_error(&e), ErrorCode::AuthFailure);
}

#[test]
fn from_provider_error_rate_limited() {
    let e = ProviderError::RateLimited {
        status: 429,
        body: "too many".into(),
    };
    assert_eq!(ErrorCode::from_provider_error(&e), ErrorCode::RateLimited);
}

#[test]
fn from_provider_error_all_seven() {
    let pairs = [
        (
            ProviderError::Auth {
                status: 401,
                body: "x".into(),
            },
            ErrorCode::AuthFailure,
        ),
        (
            ProviderError::RateLimited {
                status: 429,
                body: "x".into(),
            },
            ErrorCode::RateLimited,
        ),
        (
            ProviderError::ModelNotFound("gpt-5".into()),
            ErrorCode::ModelNotFound,
        ),
        (
            ProviderError::Network("timeout".into()),
            ErrorCode::ProviderExhausted,
        ),
        (ProviderError::Timeout, ErrorCode::ProviderExhausted),
        (
            ProviderError::Http {
                status: 503,
                body: "x".into(),
            },
            ErrorCode::ProviderExhausted,
        ),
        (
            ProviderError::Http {
                status: 400,
                body: "x".into(),
            },
            ErrorCode::ProviderFatal,
        ),
        (ProviderError::Parse("bad json".into()), ErrorCode::Unknown),
    ];
    for (pe, expected) in &pairs {
        assert_eq!(
            ErrorCode::from_provider_error(pe),
            *expected,
            "mismatch for {pe:?}"
        );
    }
}

#[test]
fn as_str_all_ten() {
    let cases = [
        (ErrorCode::Cancelled, "cancelled"),
        (ErrorCode::TaskTimeout, "task_timeout"),
        (ErrorCode::ToolFailure, "tool_failure"),
        (ErrorCode::MaxIterations, "max_iterations"),
        (ErrorCode::ProviderExhausted, "provider_exhausted"),
        (ErrorCode::ProviderFatal, "provider_fatal"),
        (ErrorCode::ModelNotFound, "model_not_found"),
        (ErrorCode::AuthFailure, "auth_failure"),
        (ErrorCode::RateLimited, "rate_limited"),
        (ErrorCode::Unknown, "unknown"),
    ];
    for (code, expected) in &cases {
        assert_eq!(code.as_str(), *expected);
    }
}

#[test]
fn legacy_strings_unchanged() {
    // P2.6 strings must be identical to ErrorCode::as_str().
    assert_eq!(ErrorCode::Cancelled.as_str(), "cancelled");
    assert_eq!(ErrorCode::TaskTimeout.as_str(), "task_timeout");
    assert_eq!(ErrorCode::ToolFailure.as_str(), "tool_failure");
    assert_eq!(ErrorCode::MaxIterations.as_str(), "max_iterations");
    assert_eq!(ErrorCode::ProviderExhausted.as_str(), "provider_exhausted");
    assert_eq!(ErrorCode::ProviderFatal.as_str(), "provider_fatal");
}

#[test]
fn serde_roundtrip() {
    let json = serde_json::to_string(&ErrorCode::TaskTimeout).unwrap();
    assert_eq!(json, "\"task_timeout\"");
    let back: ErrorCode = serde_json::from_str(&json).unwrap();
    assert_eq!(back, ErrorCode::TaskTimeout);
}

#[test]
fn serde_all_ten_roundtrip() {
    let all = [
        ErrorCode::Cancelled,
        ErrorCode::TaskTimeout,
        ErrorCode::ToolFailure,
        ErrorCode::MaxIterations,
        ErrorCode::ProviderExhausted,
        ErrorCode::ProviderFatal,
        ErrorCode::ModelNotFound,
        ErrorCode::AuthFailure,
        ErrorCode::RateLimited,
        ErrorCode::Unknown,
    ];
    for code in &all {
        let json = serde_json::to_string(code).unwrap();
        let back: ErrorCode = serde_json::from_str(&json).unwrap();
        assert_eq!(&back, code);
    }
}
