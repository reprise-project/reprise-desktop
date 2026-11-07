//! The X.224 Connection Request read before the acceptor starts, and the security protocol
//! chosen from it.

use ironrdp_pdu::nego::{ConnectionRequest, NegoRequestData, SecurityProtocol};
use ironrdp_pdu::x224::X224;

/// Characters of the user name mstsc keeps in its cookie.
const COOKIE_CHARS: usize = 9;

/// The protocols the client offers and the user its `Cookie: mstshash=` names, without a
/// `DOMAIN\` or `@domain`.
pub fn peek_request(bytes: &[u8]) -> Option<(SecurityProtocol, Option<String>)> {
    let request = ironrdp_core::decode::<X224<ConnectionRequest>>(bytes)
        .ok()?
        .0;
    let user = match request.nego_data {
        Some(NegoRequestData::Cookie(cookie)) => Some(strip_domain(&cookie.0).to_owned()),
        _ => None,
    };
    Some((request.protocol, user))
}

/// `DOM\anna` and `anna@dom` are `anna`.
pub fn strip_domain(user: &str) -> &str {
    let user = user.rsplit('\\').next().unwrap_or(user);
    user.split('@').next().unwrap_or(user)
}

/// NLA when the client offers it and its cookie names a user with a Remote Desktop password
/// (or there is no cookie and such a user exists); else TLS; else NLA when such a user exists.
pub fn choose_protocol(
    offered: SecurityProtocol,
    cookie: Option<&str>,
    nla_users: &[String],
) -> Option<SecurityProtocol> {
    let nla = if offered.contains(SecurityProtocol::HYBRID_EX) {
        Some(SecurityProtocol::HYBRID_EX)
    } else if offered.contains(SecurityProtocol::HYBRID) {
        Some(SecurityProtocol::HYBRID)
    } else {
        None
    };
    let any = !nla_users.is_empty();
    let named = match cookie {
        Some(cookie) => names_a_user(cookie, nla_users),
        None => any,
    };
    match nla {
        Some(nla) if named => Some(nla),
        _ if offered.contains(SecurityProtocol::SSL) => Some(SecurityProtocol::SSL),
        Some(nla) if any => Some(nla),
        _ => None,
    }
}

/// The cookie is one of `users`, or a truncated name that only one of them starts with.
fn names_a_user(cookie: &str, users: &[String]) -> bool {
    if users.iter().any(|u| u == cookie) {
        return true;
    }
    cookie.chars().count() == COOKIE_CHARS
        && users.iter().filter(|u| u.starts_with(cookie)).count() == 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironrdp_pdu::nego::{Cookie, RequestFlags};

    const SSL: SecurityProtocol = SecurityProtocol::SSL;
    const HYBRID: SecurityProtocol = SecurityProtocol::HYBRID;
    const HYBRID_EX: SecurityProtocol = SecurityProtocol::HYBRID_EX;

    #[test]
    fn protocol_choice_follows_the_cookie_and_the_nla_users() {
        let n: Vec<String> = vec!["anna".into(), "averylongname".into()];
        let mut ambiguous = n.clone();
        ambiguous.push("averylongother".into());
        let none: Vec<String> = Vec::new();
        let cases: [(_, _, &[String], _); 10] = [
            (SSL | HYBRID | HYBRID_EX, Some("anna"), &n, Some(HYBRID_EX)),
            (SSL | HYBRID, Some("bert"), &n, Some(SSL)),
            (SSL | HYBRID, None, &n, Some(HYBRID)),
            (SSL | HYBRID, None, &none, Some(SSL)),
            (HYBRID, Some("bert"), &n, Some(HYBRID)),
            (HYBRID, Some("bert"), &none, None),
            (SecurityProtocol::empty(), None, &n, None),
            (SSL | HYBRID, Some("averylong"), &n, Some(HYBRID)),
            (SSL | HYBRID, Some("averylong"), &ambiguous, Some(SSL)),
            (SSL | HYBRID, Some("ann"), &n, Some(SSL)),
        ];
        for (offered, cookie, users, want) in cases {
            assert_eq!(
                choose_protocol(offered, cookie, users),
                want,
                "{offered:?} with cookie {cookie:?} and users {users:?}"
            );
        }
    }

    fn request(cookie: &str) -> Vec<u8> {
        ironrdp_core::encode_vec(&X224(ConnectionRequest {
            nego_data: Some(NegoRequestData::Cookie(Cookie(cookie.into()))),
            flags: RequestFlags::empty(),
            protocol: SSL | HYBRID,
            correlation_info: None,
        }))
        .unwrap()
    }

    #[test]
    fn the_cookie_loses_its_domain() {
        for cookie in ["DOM\\anna", "anna@dom", "anna"] {
            assert_eq!(
                peek_request(&request(cookie)),
                Some((SSL | HYBRID, Some("anna".into()))),
                "{cookie}"
            );
        }
    }
}
