use anyhow::{format_err, Error};
use bytes::BytesMut;
use chrono::offset::FixedOffset;
use chrono_tz::Tz;
use derive_more::Into;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use stack_string::StackString;
use std::{convert::TryFrom, fmt, ops::Deref, str::FromStr};
use tokio_postgres::types::{FromSql, IsNull, ToSql, Type};

#[derive(Into, Debug, PartialEq, Copy, Clone, Eq, Serialize, Deserialize)]
#[serde(into = "String", try_from = "&str")]
pub struct StravaTz(Tz);

impl Deref for StravaTz {
    type Target = Tz;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<StravaTz> for String {
    fn from(item: StravaTz) -> Self {
        item.0.name().to_string()
    }
}

impl From<StravaTz> for StackString {
    fn from(item: StravaTz) -> Self {
        item.0.name().into()
    }
}

impl FromStr for StravaTz {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse()
            .map(Self)
            .map_err(|e| format_err!("{} is not a valid timezone", e))
    }
}

impl TryFrom<&str> for StravaTz {
    type Error = Error;
    fn try_from(item: &str) -> Result<Self, Self::Error> {
        item.parse()
    }
}

impl TryFrom<StackString> for StravaTz {
    type Error = Error;
    fn try_from(item: StackString) -> Result<Self, Self::Error> {
        item.as_str().parse()
    }
}

#[derive(Into, Debug, PartialEq, Copy, Clone, Eq, Serialize, Deserialize)]
#[serde(into = "String", try_from = "&str")]
pub struct StravaTimeZone(FixedOffset, Tz);

impl Default for StravaTimeZone {
    fn default() -> Self {
        Self(FixedOffset::east(0), Tz::UTC)
    }
}

impl fmt::Display for StravaTimeZone {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "(GMT{:?}) {:?}", self.0, self.1)
    }
}

impl Deref for StravaTimeZone {
    type Target = Tz;
    fn deref(&self) -> &Self::Target {
        self.tz()
    }
}

impl AsRef<Tz> for StravaTimeZone {
    fn as_ref(&self) -> &Tz {
        self.tz()
    }
}

impl AsRef<FixedOffset> for StravaTimeZone {
    fn as_ref(&self) -> &FixedOffset {
        self.offset()
    }
}

impl From<StravaTimeZone> for String {
    fn from(item: StravaTimeZone) -> Self {
        item.to_string()
    }
}

impl From<StravaTimeZone> for StackString {
    fn from(item: StravaTimeZone) -> Self {
        StackString::from_display(item)
    }
}

impl FromStr for StravaTimeZone {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_strava_str(s)
    }
}

impl TryFrom<&str> for StravaTimeZone {
    type Error = Error;
    fn try_from(item: &str) -> Result<Self, Self::Error> {
        item.parse()
    }
}

impl StravaTimeZone {
    pub fn tz(&self) -> &Tz {
        &self.1
    }

    pub fn offset(&self) -> &FixedOffset {
        &self.0
    }

    pub fn from_strava_str(s: &str) -> Result<Self, Error> {
        let mut offset = None;
        let tz_strs: SmallVec<[&str; 2]> = s.split_whitespace().take(2).collect();
        if let Some(tz) = tz_strs.get(0) {
            if tz.get(1..=3) != Some("GMT") {
                return Err(format_err!("Time string isn't GMT: {}", tz));
            }
            if let Some(hours) = tz.get(4..=6).and_then(|s| s.parse::<i32>().ok()) {
                if let Some(minutes) = tz.get(8..=9).and_then(|s| s.parse::<i32>().ok()) {
                    offset.replace(FixedOffset::east(hours * 3600 + minutes * 60));
                }
            }
        }
        if let Some(tz) = tz_strs.get(1) {
            let tz: Tz = tz
                .parse()
                .map_err(|e| format_err!("{} is not a valid timezone", e))?;
            let offset = offset.ok_or_else(|| format_err!("Bad offset"))?;
            Ok(Self(offset, tz))
        } else {
            Err(format_err!("Bad Timezone String"))
        }
    }
}

impl<'a> FromSql<'a> for StravaTimeZone {
    fn from_sql(
        ty: &Type,
        raw: &'a [u8],
    ) -> Result<Self, Box<dyn std::error::Error + 'static + Send + Sync>> {
        let s = String::from_sql(ty, raw)?.parse()?;
        Ok(s)
    }

    fn accepts(ty: &Type) -> bool {
        <String as FromSql>::accepts(ty)
    }
}

impl ToSql for StravaTimeZone {
    fn to_sql(
        &self,
        ty: &Type,
        out: &mut BytesMut,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>>
    where
        Self: Sized,
    {
        let s = StackString::from_display(self);
        s.to_sql(ty, out)
    }

    fn accepts(ty: &Type) -> bool
    where
        Self: Sized,
    {
        <String as ToSql>::accepts(ty)
    }

    fn to_sql_checked(
        &self,
        ty: &Type,
        out: &mut BytesMut,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        let s = StackString::from_display(self);
        s.to_sql_checked(ty, out)
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Error;
    use stack_string::format_sstr;
    use std::fmt::Write;

    use crate::common::strava_timezone::StravaTimeZone;

    #[test]
    fn test_timezone() -> Result<(), Error> {
        let timezones = [
            "(GMT+01:00) Europe/Amsterdam",
            "(GMT+01:00) Europe/Berlin",
            "(GMT-05:00) America/New_York",
            "(GMT-06:00) America/Chicago",
            "(GMT-07:00) America/Denver",
            "(GMT-08:00) America/Los_Angeles",
        ];
        for tz in timezones.iter() {
            let stz: StravaTimeZone = tz.parse()?;
            assert_eq!(&format_sstr!("{}", stz).as_str(), tz);
        }
        Ok(())
    }
}
