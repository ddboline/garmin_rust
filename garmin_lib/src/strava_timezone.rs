use bytes::BytesMut;
use derive_more::Into;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use stack_string::{format_sstr, StackString};
use std::{convert::TryFrom, fmt, ops::Deref, str::FromStr};
use time::UtcOffset;
use time_tz::{
    timezones::{db::UTC, get_by_name},
    TimeZone, Tz,
};
use tokio_postgres::types::{FromSql, IsNull, ToSql, Type};

use crate::errors::GarminError as Error;

#[derive(Into, Debug, PartialEq, Copy, Clone, Eq, Serialize, Deserialize)]
#[serde(into = "String", try_from = "&str")]
pub struct StravaTz(&'static Tz);

impl Deref for StravaTz {
    type Target = Tz;
    fn deref(&self) -> &Self::Target {
        self.0
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
        get_by_name(s)
            .map(Self)
            .ok_or_else(|| Error::CustomError(format_sstr!("{s} is not a valid timezone")))
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
pub struct StravaTimeZone(UtcOffset, &'static Tz);

impl Default for StravaTimeZone {
    fn default() -> Self {
        Self(
            UtcOffset::from_whole_seconds(0).unwrap_or(UtcOffset::UTC),
            UTC,
        )
    }
}

impl fmt::Display for StravaTimeZone {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let (h, m, _) = self.0.as_hms();
        write!(
            f,
            "(GMT{s}{h:02}:{m:02}) {t}",
            s = if self.0.is_negative() { '-' } else { '+' },
            h = h.abs(),
            m = m.abs(),
            t = self.1.name(),
        )
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

impl AsRef<UtcOffset> for StravaTimeZone {
    fn as_ref(&self) -> &UtcOffset {
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
    #[must_use]
    pub fn tz(&self) -> &Tz {
        self.1
    }

    #[must_use]
    pub fn offset(&self) -> &UtcOffset {
        &self.0
    }

    /// # Errors
    /// Return error if parsing timezone fails
    pub fn from_strava_str(s: &str) -> Result<Self, Error> {
        let mut offset = None;
        let tz_strs: SmallVec<[&str; 2]> = s.split_whitespace().take(2).collect();
        if let Some(tz) = tz_strs.first() {
            if tz.get(1..=3) != Some("GMT") {
                return Err(Error::CustomError(format_sstr!(
                    "Time string isn't GMT: {tz}"
                )));
            }
            if let Some(hours) = tz.get(4..=6).and_then(|s| s.parse::<i32>().ok()) {
                if let Some(minutes) = tz.get(8..=9).and_then(|s| s.parse::<i32>().ok()) {
                    offset.replace(
                        UtcOffset::from_whole_seconds(hours * 3600 + minutes * 60)
                            .unwrap_or(UtcOffset::UTC),
                    );
                }
            }
        }
        if let Some(tz) = tz_strs.get(1) {
            let tz = get_by_name(tz)
                .ok_or_else(|| Error::CustomError(format_sstr!("{tz} is not a valid timezone")))?;
            let offset = offset.ok_or_else(|| Error::StaticCustomError("Bad offset"))?;
            Ok(Self(offset, tz))
        } else {
            Err(Error::StaticCustomError("Bad Timezone String"))
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
    use stack_string::format_sstr;

    use crate::{errors::GarminError as Error, strava_timezone::StravaTimeZone};

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
        for tz in timezones {
            let stz: StravaTimeZone = tz.parse()?;
            assert_eq!(format_sstr!("{stz}").as_str(), tz);
        }
        Ok(())
    }
}
