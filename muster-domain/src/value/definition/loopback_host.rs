//! The names of this machine a probe URL may use, so a probe never leaves
//! it.

use std::fmt::Display;

use strum::{AsRefStr, Display, EnumIter, IntoEnumIterator};

use crate::text::{ListForm, ValueList};

/// The one scheme a URL on this machine is written with: muster serves and
/// probes no TLS.
pub const HTTP_SCHEME: &str = "http://";
/// Comes before the hosts in the form a probe URL is written in.
const URL_FORM_LEAD: &str = "an http URL on";

/// A loopback host, as a URL's authority names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, AsRefStr, Display, EnumIter)]
#[strum(serialize_all = "kebab-case")]
pub enum LoopbackHost {
    /// The name every system resolves to loopback.
    #[strum(serialize = "localhost")]
    Localhost,
    /// The IPv4 loopback address.
    #[strum(serialize = "127.0.0.1")]
    Ipv4,
    /// The IPv6 loopback address, bracketed as in a URL.
    #[strum(serialize = "[::1]")]
    Ipv6,
}

impl LoopbackHost {
    /// Whether `host` is this one, ignoring case.
    pub fn is(self, host: &str) -> bool {
        host.eq_ignore_ascii_case(self.as_ref())
    }

    /// Whether `host` is any loopback host, ignoring case.
    pub fn any_is(host: &str) -> bool {
        Self::iter().any(|loopback| loopback.is(host))
    }

    /// The URL of `port` on this host: `http://localhost:3000`.
    pub fn http_url(self, port: impl Display) -> String {
        format!("{HTTP_SCHEME}{self}:{port}")
    }

    /// The form a probe URL is written in, as a refusal states it:
    /// `an http URL on localhost, 127.0.0.1 or [::1]`.
    pub fn url_form() -> String {
        format!(
            "{URL_FORM_LEAD} {}",
            ValueList::of::<Self>().written(ListForm::Either)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every loopback name reads in any case, the form lists them all, and
    /// another host is none of them.
    #[test]
    fn loopback_hosts_read_in_any_case_and_list_in_the_form() {
        assert!(LoopbackHost::any_is("LocalHost"));
        assert!(LoopbackHost::any_is("127.0.0.1"));
        assert!(LoopbackHost::any_is("[::1]"));
        assert!(!LoopbackHost::any_is("example.com"));
        assert!(!LoopbackHost::any_is("::1"));
        assert_eq!(
            LoopbackHost::Localhost.http_url(3000),
            "http://localhost:3000"
        );
        assert_eq!(
            LoopbackHost::url_form(),
            "an http URL on localhost, 127.0.0.1 or [::1]"
        );
    }
}
