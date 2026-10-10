//! Unblocks the Bluetooth radios through `/dev/rfkill`, so turning Bluetooth on
//! works after a soft block (an airplane-mode key, `rfkill block`). The seat's
//! user can write the device through systemd's uaccess rule. A hard block stays.

use std::io::Write;

const TYPE_BLUETOOTH: u8 = 2;
const OP_CHANGE_ALL: u8 = 3;

/// A `struct rfkill_event` in its first, 8-byte layout, which every
/// kernel reads: idx (u32), type, op, soft, hard.
fn unblock_event() -> [u8; 8] {
    let mut ev = [0u8; 8];
    ev[4] = TYPE_BLUETOOTH;
    ev[5] = OP_CHANGE_ALL;
    ev
}

pub fn unblock() -> std::io::Result<()> {
    std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/rfkill")?
        .write_all(&unblock_event())
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_unblock_event_names_bluetooth_and_change_all() {
        // idx 0 (ignored by CHANGE_ALL), type 2, op 3, soft 0, hard 0.
        assert_eq!(super::unblock_event(), [0, 0, 0, 0, 2, 3, 0, 0]);
    }
}
