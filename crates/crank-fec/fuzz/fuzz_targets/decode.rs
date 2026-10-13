#![no_main]

libfuzzer_sys::fuzz_target!(|bytes: &[u8]| {
    let _ = crank_fec::decode(bytes);
    // Also explore every page parser behind a valid envelope/checksum.
    if let Ok(page) = <&[u8; 8]>::try_from(bytes) {
        let mut frame = [0xa4, 9, 0x4e, 5, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        frame[4..12].copy_from_slice(page);
        frame[12] = frame[..12].iter().fold(0, |a, b| a ^ b);
        assert!(crank_fec::decode(&frame).is_ok());
    }
});
