//! Geotag photos from a GPX track recorded on a phone or GPS (most cameras have no GPS).

use anyhow::{Result, bail};
use chrono::{DateTime, Utc};
use quick_xml::events::Event;
use quick_xml::{Reader, XmlVersion};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackPoint {
    pub time: DateTime<Utc>,
    pub lat: f64,
    pub lon: f64,
    pub ele: Option<f64>,
}

/// Parse track, route and waypoint points that carry a timestamp; sorted by time.
pub fn parse(xml: &str) -> Result<Vec<TrackPoint>> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut points = Vec::new();
    let mut cur: Option<(f64, f64)> = None;
    let (mut time, mut ele): (Option<DateTime<Utc>>, Option<f64>) = (None, None);
    let mut field: Option<&'static str> = None;
    loop {
        match reader.read_event()? {
            Event::Start(e) | Event::Empty(e) => {
                let name = e.local_name();
                match name.into_inner() {
                    "trkpt" | "rtept" | "wpt" => {
                        let (mut lat, mut lon) = (None, None);
                        for a in e.attributes().flatten() {
                            let v = a
                                .normalized_value(XmlVersion::Implicit1_0)?
                                .parse::<f64>()
                                .ok();
                            match a.key.local_name().into_inner() {
                                "lat" => lat = v,
                                "lon" => lon = v,
                                _ => {}
                            }
                        }
                        cur = lat.zip(lon);
                        (time, ele) = (None, None);
                    }
                    "time" if cur.is_some() => field = Some("time"),
                    "ele" if cur.is_some() => field = Some("ele"),
                    _ => {}
                }
            }
            Event::Text(t) => {
                let text = t.xml_content(XmlVersion::Implicit1_0);
                match field {
                    Some("time") => {
                        time = DateTime::parse_from_rfc3339(text.trim())
                            .ok()
                            .map(|d| d.to_utc())
                    }
                    Some("ele") => ele = text.trim().parse().ok(),
                    _ => {}
                }
            }
            Event::End(e) => {
                match e.local_name().into_inner() {
                    "trkpt" | "rtept" | "wpt" => {
                        if let (Some((lat, lon)), Some(t)) = (cur.take(), time) {
                            points.push(TrackPoint {
                                time: t,
                                lat,
                                lon,
                                ele,
                            });
                        }
                    }
                    _ => {}
                }
                field = None;
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if points.is_empty() {
        bail!("no timestamped points found in the GPX file");
    }
    points.sort_by_key(|p| p.time);
    Ok(points)
}

/// Position at `t`: interpolated between the surrounding points when both are within
/// `max_gap_s`, else the nearest point if within `max_gap_s`.
pub fn locate(track: &[TrackPoint], t: DateTime<Utc>, max_gap_s: i64) -> Option<TrackPoint> {
    let i = track.partition_point(|p| p.time <= t);
    let before = i.checked_sub(1).map(|j| track[j]);
    let after = track.get(i).copied();
    let gap = |p: &TrackPoint| (p.time - t).num_seconds().abs();
    match (before, after) {
        (Some(a), Some(b)) if gap(&a) <= max_gap_s && gap(&b) <= max_gap_s => {
            let span = (b.time - a.time).num_milliseconds() as f64;
            let f = if span > 0.0 {
                (t - a.time).num_milliseconds() as f64 / span
            } else {
                0.0
            };
            Some(TrackPoint {
                time: t,
                lat: a.lat + (b.lat - a.lat) * f,
                lon: a.lon + (b.lon - a.lon) * f,
                ele: a.ele.zip(b.ele).map(|(x, y)| x + (y - x) * f).or(a.ele),
            })
        }
        (a, b) => [a, b]
            .into_iter()
            .flatten()
            .filter(|p| gap(p) <= max_gap_s)
            .min_by_key(gap),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GPX: &str = r#"<?xml version="1.0"?>
<gpx version="1.1" xmlns="http://www.topografix.com/GPX/1/1"><trk><trkseg>
  <trkpt lat="39.0" lon="-105.0"><ele>1600</ele><time>2026-05-03T14:00:00Z</time></trkpt>
  <trkpt lat="39.1" lon="-105.2"><ele>1700</ele><time>2026-05-03T14:10:00Z</time></trkpt>
  <trkpt lat="40.0" lon="-104.0"><time>2026-05-03T18:00:00Z</time></trkpt>
</trkseg></trk></gpx>"#;

    #[test]
    fn parses_and_interpolates() {
        let tr = parse(GPX).unwrap();
        assert_eq!(tr.len(), 3);
        let t = DateTime::parse_from_rfc3339("2026-05-03T14:05:00Z")
            .unwrap()
            .to_utc();
        let p = locate(&tr, t, 900).unwrap();
        assert!((p.lat - 39.05).abs() < 1e-9 && (p.lon + 105.1).abs() < 1e-9);
        assert_eq!(p.ele, Some(1650.0));
    }

    #[test]
    fn respects_max_gap() {
        let tr = parse(GPX).unwrap();
        let t = DateTime::parse_from_rfc3339("2026-05-03T16:00:00Z")
            .unwrap()
            .to_utc();
        assert!(locate(&tr, t, 900).is_none());
        let near_end = DateTime::parse_from_rfc3339("2026-05-03T18:05:00Z")
            .unwrap()
            .to_utc();
        assert_eq!(locate(&tr, near_end, 900).unwrap().lat, 40.0);
    }
}
