#![no_main]
use anise::frames::Frame;
use anise::structure::planetocentric::ellipsoid::Ellipsoid;

use libfuzzer_sys::fuzz_target;


fn do_fuzz_from_name(data: &[u8]) {
    // Ensure the data is long enough for function being tested
    if data.len() < 2 {
        return;
    }

    // Use the first half as the center and the second half as the ref_frame
    let data_middle = data.len() / 2;
    let center = std::str::from_utf8(&data[..data_middle]).unwrap_or("");
    let ref_frame = std::str::from_utf8(&data[data_middle..]).unwrap_or("");
    
    let _ = Frame::from_name(center, ref_frame);
}

fn do_fuzz_with_ellipsoid(data: &[u8]) {
    // Ensure the data is long enough for function being tested
    if data.len() < 32 {
        return;
    }

    // Build a frame based on ids using the first parts of data
    let ephemeris_id = i32::from_le_bytes(data[0..4].try_into().unwrap());
    let orientation_id = i32::from_le_bytes(data[4..8].try_into().unwrap());
    let frame = Frame::new(ephemeris_id, orientation_id);

    // Build an ellipsoid from the remaining data
    let semi_major_equatorial_radius_km = f64::from_le_bytes(data[8..16].try_into().unwrap());
    let semi_minor_equatorial_radius_km = f64::from_le_bytes(data[16..24].try_into().unwrap());
    let polar_radius_km = f64::from_le_bytes(data[24..32].try_into().unwrap());
    let ellipsoid = Ellipsoid {
        semi_major_equatorial_radius_km,
        semi_minor_equatorial_radius_km,
        polar_radius_km,
    };
    
    let _ = frame.with_ellipsoid(ellipsoid);
}

fuzz_target!(|data: &[u8]| {
    do_fuzz_from_name(data);
    do_fuzz_with_ellipsoid(data);
});
