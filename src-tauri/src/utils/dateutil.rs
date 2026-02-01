use chrono::{NaiveDate, FixedOffset, TimeZone, NaiveDateTime, DateTime, Local};
use serde::{Deserialize, Deserializer, Serializer};

pub fn date2timestamp(date: &str) -> i64 {
    let date_str = if date.len() > 10 { &date[..10] } else { date };
    let dt = NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
        .expect(&format!("Failed to parse date: {}", date))
        .and_hms_opt(0, 0, 0).unwrap();
    let tz = FixedOffset::east_opt(8 * 3600).unwrap();
    tz.from_local_datetime(&dt).unwrap().timestamp_millis()
}

pub fn serialize_datetime<S>(dt: &NaiveDateTime, serializer: S) -> Result<S::Ok, S::Error>
where S: Serializer {
    serializer.serialize_str(&dt.format("%Y-%m-%d %H:%M:%S").to_string())
}

pub fn deserialize_datetime<'de, D>(deserializer: D) -> Result<NaiveDateTime, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    // 尝试解析带时区的 ISO 8601 格式 (如 JS Date.toISOString())
    if let Ok(dt) = DateTime::parse_from_rfc3339(&s) {
        return Ok(dt.with_timezone(&Local).naive_local());
    }
    // 尝试解析不带时区的格式 (如 serialize_naive_datetime 输出的格式)
    if let Ok(dt) = NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S") {
        return Ok(dt);
    }
    // 尝试解析带T的不带时区格式
    if let Ok(dt) = NaiveDateTime::parse_from_str(&s, "%Y-%m-%dT%H:%M:%S") {
        return Ok(dt);
    }
    
    Err(serde::de::Error::custom(format!("Invalid datetime format: {}", s)))
}

pub fn serialize_option_datetime<S>(dt: &Option<NaiveDateTime>, serializer: S) -> Result<S::Ok, S::Error>
where S: Serializer {
    match dt {
        Some(d) => serializer.serialize_str(&d.format("%Y-%m-%d %H:%M:%S").to_string()),
        None => serializer.serialize_none(),
    }
}

pub fn deserialize_option_datetime<'de, D>(deserializer: D) -> Result<Option<NaiveDateTime>, D::Error>
where
    D: Deserializer<'de>,
{
    let s: Option<String> = Option::deserialize(deserializer)?;
    match s {
        Some(s) => {
            if s.is_empty() {
                return Ok(None);
            }
            if let Ok(dt) = DateTime::parse_from_rfc3339(&s) {
                return Ok(Some(dt.with_timezone(&Local).naive_local()));
            }
            if let Ok(dt) = NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S") {
                return Ok(Some(dt));
            }
            if let Ok(dt) = NaiveDateTime::parse_from_str(&s, "%Y-%m-%dT%H:%M:%S") {
                return Ok(Some(dt));
            }
            Err(serde::de::Error::custom(format!("Invalid datetime format: {}", s)))
        },
        None => Ok(None)
    }
}
