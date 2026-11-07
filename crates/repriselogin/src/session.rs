//! Messages a user session sends reprise-login over `SESSION_SOCKET`.

use serde::{Deserialize, Serialize};

use crate::Interface;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionRequest {
    Hello { interface: Interface },
    SwitchUser,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::encode;

    #[test]
    fn wire_shapes_are_pinned() {
        let body = |m: &SessionRequest| encode(m).unwrap()[4..].to_vec();
        assert_eq!(
            body(&SessionRequest::Hello {
                interface: Interface::Glass
            }),
            br#"{"type":"hello","interface":"glass"}"#
        );
        assert_eq!(
            body(&SessionRequest::SwitchUser),
            br#"{"type":"switch_user"}"#
        );
    }
}
