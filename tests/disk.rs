use super::*;

#[test]
fn a_disk_whose_name_ends_in_a_digit_puts_p_before_the_number() {
    assert_eq!(partition("/dev/nvme0n1", 2), "/dev/nvme0n1p2");
    assert_eq!(partition("/dev/mmcblk0", 1), "/dev/mmcblk0p1");
    assert_eq!(partition("/dev/sda", 3), "/dev/sda3");
    assert_eq!(partition("/dev/vda", 1), "/dev/vda1");
}
