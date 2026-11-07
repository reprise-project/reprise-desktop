//! What a client is told for each way this side ends it.

use super::*;

fn code(why: ErrorInfo) -> u32 {
    match why {
        ErrorInfo::ProtocolIndependentCode(code) => code as u32,
        other => panic!("{other:?}"),
    }
}

#[test]
fn each_kick_tells_its_reason() {
    let told = [
        (Kick::Asked, 0xB),
        (Kick::PortChanged, 0xB),
        (Kick::Stopped, 0x1),
        (Kick::Replaced, 0x5),
        (Kick::LoggedOff, 0xC),
    ];
    for (kick, expected) in told {
        assert_eq!(code(told_for(kick)), expected, "{kick:?}");
    }
    assert_eq!(code(TOOK_OVER), 0x5, "an unlocked console tells 0x5");
}
