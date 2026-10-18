//! Capture metadata: JPEG/HEIC via nom-exif, camera RAW via rawler.

use std::path::Path;

use chrono::{DateTime, FixedOffset, Local, NaiveDateTime, TimeZone, Utc};
use nom_exif::{Exif, ExifTag, MediaParser, MediaSource, read_exif};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Meta {
    pub captured_at: Option<DateTime<Utc>>,
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub lens: Option<String>,
    pub focal_mm: Option<f64>,
    pub aperture: Option<f64>,
    pub shutter_s: Option<f64>,
    pub iso: Option<i32>,
    /// EXIF orientation (1..=8).
    pub orientation: Option<u8>,
    pub gps_lat: Option<f64>,
    pub gps_lon: Option<f64>,
    pub gps_alt: Option<f64>,
}

impl Meta {
    /// Fill gaps in `self` from `other` (e.g. JPEG twin missing a lens name the RAW has).
    pub fn fill_from(&mut self, other: Meta) {
        macro_rules! fill {
            ($($f:ident),*) => { $( if self.$f.is_none() { self.$f = other.$f; } )* };
        }
        fill!(
            captured_at,
            camera_make,
            camera_model,
            lens,
            focal_mm,
            aperture,
            shutter_s,
            iso,
            orientation,
            gps_lat,
            gps_lon,
            gps_alt
        );
    }
}

/// How to interpret EXIF times that carry no UTC offset (most cameras).
#[derive(Debug, Clone)]
pub enum CameraClock {
    SystemLocal,
    Zone(chrono_tz::Tz),
}

impl CameraClock {
    pub fn from_config(tz: Option<&str>) -> Self {
        match tz.and_then(|t| t.parse::<chrono_tz::Tz>().ok()) {
            Some(z) => Self::Zone(z),
            None => Self::SystemLocal,
        }
    }

    fn to_utc(&self, naive: NaiveDateTime) -> Option<DateTime<Utc>> {
        match self {
            Self::SystemLocal => Local
                .from_local_datetime(&naive)
                .earliest()
                .map(|t| t.to_utc()),
            Self::Zone(z) => z.from_local_datetime(&naive).earliest().map(|t| t.to_utc()),
        }
    }
}

fn clean(s: Option<&str>) -> Option<String> {
    let s = s?.trim().trim_matches('\0').trim();
    (!s.is_empty()).then(|| s.to_string())
}

fn parse_offset(s: &str) -> Option<FixedOffset> {
    DateTime::parse_from_str(
        &format!("2000-01-01T00:00:00{}", s.trim()),
        "%Y-%m-%dT%H:%M:%S%:z",
    )
    .ok()
    .map(|d| *d.offset())
}

fn parse_exif_time(s: &str, offset: Option<&str>, clock: &CameraClock) -> Option<DateTime<Utc>> {
    let naive = NaiveDateTime::parse_from_str(s.trim(), "%Y:%m:%d %H:%M:%S").ok()?;
    match offset.and_then(parse_offset) {
        Some(off) => off.from_local_datetime(&naive).single().map(|t| t.to_utc()),
        None => clock.to_utc(naive),
    }
}

/// Metadata from a JPEG/HEIC/TIFF-like file.
pub fn read_image_meta(path: &Path, clock: &CameraClock) -> anyhow::Result<Meta> {
    Ok(meta_from_exif(&read_exif(path)?, clock))
}

/// Metadata from in-memory JPEG/HEIC bytes (e.g. an original fetched from object storage).
pub fn read_image_meta_bytes(bytes: bytes::Bytes, clock: &CameraClock) -> anyhow::Result<Meta> {
    let exif: Exif = MediaParser::new()
        .parse_exif(MediaSource::from_memory(bytes)?)?
        .into();
    Ok(meta_from_exif(&exif, clock))
}

fn meta_from_exif(exif: &Exif, clock: &CameraClock) -> Meta {
    let get = |t: ExifTag| exif.get(t);
    let text = |t: ExifTag| clean(get(t).and_then(|v| v.as_str()));
    let float = |t: ExifTag| {
        get(t)
            .and_then(|v| v.try_as_float())
            .filter(|v| v.is_finite() && *v > 0.0)
    };

    let captured_at = match get(ExifTag::DateTimeOriginal).and_then(|v| v.as_datetime()) {
        Some(dt) => match dt.aware() {
            Some(aware) => Some(aware.to_utc()),
            None => clock.to_utc(dt.into_naive()),
        },
        None => text(ExifTag::DateTimeOriginal)
            .or_else(|| text(ExifTag::CreateDate))
            .and_then(|s| parse_exif_time(&s, text(ExifTag::OffsetTimeOriginal).as_deref(), clock)),
    };

    let gps = exif.gps_info();
    Meta {
        captured_at,
        camera_make: text(ExifTag::Make),
        camera_model: text(ExifTag::Model),
        lens: text(ExifTag::LensModel),
        focal_mm: float(ExifTag::FocalLength),
        aperture: float(ExifTag::FNumber),
        shutter_s: float(ExifTag::ExposureTime),
        iso: get(ExifTag::ISOSpeedRatings)
            .and_then(|v| v.try_as_integer())
            .and_then(|v| i32::try_from(v).ok()),
        orientation: get(ExifTag::Orientation)
            .and_then(|v| v.try_as_integer())
            .and_then(|v| u8::try_from(v).ok()),
        gps_lat: gps
            .and_then(|g| g.latitude_decimal())
            .filter(|v| v.is_finite() && *v != 0.0),
        gps_lon: gps
            .and_then(|g| g.longitude_decimal())
            .filter(|v| v.is_finite() && *v != 0.0),
        gps_alt: gps.and_then(|g| g.altitude.meters()),
    }
}

fn rational(r: &rawler::formats::tiff::Rational) -> Option<f64> {
    (r.d != 0).then(|| r.n as f64 / r.d as f64)
}

fn dms(v: &[rawler::formats::tiff::Rational; 3], reference: Option<&str>) -> Option<f64> {
    let deg = rational(&v[0])? + rational(&v[1])? / 60.0 + rational(&v[2])? / 3600.0;
    let sign = match reference.map(str::trim) {
        Some("S") | Some("W") => -1.0,
        _ => 1.0,
    };
    (deg != 0.0).then_some(deg * sign)
}

/// Metadata from a camera RAW file (ORF, CR3, NEF, ARW, RAF, DNG, ...).
pub fn read_raw_meta(path: &Path, clock: &CameraClock) -> anyhow::Result<Meta> {
    let src = rawler::rawsource::RawSource::new(path)?;
    let decoder = rawler::get_decoder(&src)?;
    let md = decoder.raw_metadata(&src, &rawler::decoders::RawDecodeParams::default())?;
    let ex = &md.exif;
    let gps = ex.gps.as_ref();
    Ok(Meta {
        captured_at: ex
            .date_time_original
            .as_deref()
            .and_then(|s| parse_exif_time(s, ex.offset_time_original.as_deref(), clock)),
        camera_make: clean(Some(&md.make)),
        camera_model: clean(Some(&md.model)),
        lens: clean(ex.lens_model.as_deref()),
        focal_mm: ex.focal_length.as_ref().and_then(rational),
        aperture: ex.fnumber.as_ref().and_then(rational),
        shutter_s: ex.exposure_time.as_ref().and_then(rational),
        iso: ex
            .iso_speed_ratings
            .map(i32::from)
            .or(ex.iso_speed.and_then(|v| i32::try_from(v).ok())),
        orientation: ex.orientation.and_then(|o| u8::try_from(o).ok()),
        gps_lat: gps.and_then(|g| dms(g.gps_latitude.as_ref()?, g.gps_latitude_ref.as_deref())),
        gps_lon: gps.and_then(|g| dms(g.gps_longitude.as_ref()?, g.gps_longitude_ref.as_deref())),
        gps_alt: gps.and_then(|g| {
            let a = rational(g.gps_altitude.as_ref()?)?;
            Some(if g.gps_altitude_ref == Some(1) { -a } else { a })
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exif_time_with_offset() {
        let t = parse_exif_time(
            "2026:05:03 10:00:00",
            Some("-04:00"),
            &CameraClock::SystemLocal,
        )
        .unwrap();
        assert_eq!(t.to_rfc3339(), "2026-05-03T14:00:00+00:00");
    }

    #[test]
    fn exif_time_with_configured_zone() {
        let clock = CameraClock::from_config(Some("America/Denver"));
        let t = parse_exif_time("2026:01:15 09:30:00", None, &clock).unwrap();
        assert_eq!(t.to_rfc3339(), "2026-01-15T16:30:00+00:00");
    }

    #[test]
    fn fill_prefers_existing() {
        let mut a = Meta {
            lens: None,
            iso: Some(200),
            ..Default::default()
        };
        a.fill_from(Meta {
            lens: Some("M.Zuiko 150-400".into()),
            iso: Some(400),
            ..Default::default()
        });
        assert_eq!(a.lens.as_deref(), Some("M.Zuiko 150-400"));
        assert_eq!(a.iso, Some(200));
    }
}
