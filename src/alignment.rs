
pub fn align_up(value: u64, align: u64) -> u64 {
    assert!(align > 0);
    assert!(value <= u64::MAX - align);

    if value % align == 0 { value }
    else { value + (align - (value % align)) }
}

pub fn align_down(value: u64, align: u64) -> u64 {
    assert!(align > 0);

    if value % align == 0 { value }
    else { value - (value % align) }
}
