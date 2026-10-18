//! Offline reverse geocoding (GeoNames cities, bundled with the binary).

use std::sync::OnceLock;

use reverse_geocoder::ReverseGeocoder;

fn geocoder() -> &'static ReverseGeocoder {
    static G: OnceLock<ReverseGeocoder> = OnceLock::new();
    G.get_or_init(ReverseGeocoder::new)
}

/// "Town, Region, CC" for the nearest populated place.
pub fn place_name(lat: f64, lon: f64) -> Option<String> {
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return None;
    }
    let r = geocoder().search((lat, lon)).record;
    let parts: Vec<&str> = [r.name.as_str(), r.admin1.as_str(), r.cc.as_str()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect();
    (!parts.is_empty()).then(|| parts.join(", "))
}

#[cfg(test)]
mod tests {
    #[test]
    fn finds_a_city() {
        let p = super::place_name(39.9526, -75.1652).unwrap();
        assert!(p.ends_with("US"), "{p}");
    }
}
