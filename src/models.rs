use super::schema::omikujis;
use serde::{Deserialize, Serialize};
use strum_macros::EnumIter;
use strum_macros::EnumString;

#[derive(Queryable, Identifiable, Debug)]
pub struct Omikuji {
    pub id: u32,
    pub photo: Option<String>,
    pub message: String,
    pub vote_count: i32,
    pub tg_id: i64,
    pub tg_name: String,
    pub updated_at: chrono::NaiveDateTime,
    pub created_at: chrono::NaiveDateTime,
}

#[derive(Insertable)]
#[table_name = "omikujis"]
pub struct NewOmikuji<'a> {
    pub message: &'a str,
    pub tg_id: i64,
    pub tg_name: &'a str,
}

// Ref: https://en.wikipedia.org/wiki/O-mikuji (ordered by the extent of fortune)
// Great blessing (大吉, dai-kichi)
// Middle blessing (中吉, chū-kichi)
// Small blessing (小吉, shō-kichi)
// Blessing (吉, kichi)
// Half-blessing (半吉, han-kichi)
// Future blessing (末吉, sue-kichi)
// Future small blessing (末小吉, sue-shō-kichi)
// Curse (凶, kyō)
// Small curse (小凶, shō-kyō)
// Half-curse (半凶, han-kyō)
// Future curse (末凶, sue-kyō)
// Great curse (大凶, dai-kyō)
#[derive(Serialize, Deserialize, EnumIter, EnumString, Debug)]
pub enum OmikujiClass {
    GreatBlessing,
    MiddleBlessing,
    SmallBlessing,
    Blessing,
    HalfBlessing,
    FutureBlessing,
    FutureSmallBlessing,
    Curse,
    SmallCurse,
    HalfCurse,
    FutureCurse,
    GreatCurse,
    // Default class, indicating the class is not selected
    Unknown,
}

// Ref: https://en.wikipedia.org/wiki/O-mikuji (only selected part of the more relevant ones)
// hōgaku (方角) - auspicious/inauspicious directions (see feng shui)
// negaigoto (願事) – one's wish or desire
// machibito (待人) – a person being waited for
// usemono (失せ物) – lost article(s)
// tabidachi (旅立ち) – travel
// akinai (商い) – business dealings
// gakumon (学問) – studies or learning
// arasoigoto (争事) – disputes
// ren'ai (恋愛) – romantic relationships
// byōki (病気) – illness
//
// <IGNORED> sōba (相場) – market speculation
// <IGNORED> tenkyo (転居) – moving or changing residence
// <IGNORED> shussan (出産) – childbirth, delivery
// <IGNORED> endan (縁談) – marriage proposal or engagement
#[derive(Serialize, Deserialize, EnumIter, EnumString, Debug)]
pub enum OmikujiSection {
    // predefined titles, with the String being explanation
    FortuneDirection,
    Desire,
    PersonWaitedFor,
    LostArticle,
    Travel,
    Business,
    Study,
    Dispute,
    Love,
    Illness,
    Other,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct OmikujiMessage {
    pub class: OmikujiClass,
    pub sections: Vec<(OmikujiSection, String)>,
}
